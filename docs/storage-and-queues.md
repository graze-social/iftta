# Storage and Queue Configuration Guide

This guide covers all storage backends and queue adapters available in ifthisthenat, including configuration options, performance characteristics, and use case recommendations.

## Table of Contents

- [Overview](#overview)
- [Database Storage](#database-storage)
- [Queue Adapters](#queue-adapters)
- [Cursor Storage](#cursor-storage)
- [Evaluation Result Storage](#evaluation-result-storage)
- [Denylist Storage](#denylist-storage)
- [Webhook Task System](#webhook-task-system)
- [Performance Comparison](#performance-comparison)
- [Deployment Recommendations](#deployment-recommendations)

## Overview

ifthisthenat uses multiple storage systems for different purposes:

1. **Primary Database** (PostgreSQL): Blueprint definitions, application state
2. **Queue Systems**: Blueprint evaluation, webhook processing
3. **Cursor Storage**: Jetstream consumption position
4. **Result Storage**: Evaluation audit logs
5. **Cache Layer** (Redis): Distributed operations, temporary data
6. **Denylist Storage**: Content filtering and blocking

## Database Storage

### PostgreSQL (Primary)

PostgreSQL is the primary database for persistent application state.

#### Configuration

```bash
# Connection settings
DATABASE_URL=postgres://user:password@host:5432/database
DATABASE_MAX_CONNECTIONS=20    # Default: 10
DATABASE_MIN_CONNECTIONS=5     # Default: 1
```

#### Schema Overview

Key tables:
- `blueprints`: Blueprint definitions and metadata
- `nodes`: Individual blueprint node configurations
- `prototypes`: Blueprint templates with placeholders
- `prototype_instantiations`: Prototype usage tracking
- `denylist`: Content filtering rules
- `scheduler_state`: Periodic entry node execution state
- `webhook_tasks`: Webhook delivery queue (if using PostgreSQL adapter)

#### Connection Pooling

The application uses SQLx with connection pooling:

```rust
// Configured via environment variables
let pool = PgPoolOptions::new()
    .max_connections(config.database.max_connections)
    .min_connections(config.database.min_connections)
    .connect(&config.database_url)
    .await?;
```

#### Performance Optimization

```sql
-- Recommended PostgreSQL settings for production
# postgresql.conf
shared_buffers = 256MB
effective_cache_size = 1GB
maintenance_work_mem = 64MB
checkpoint_completion_target = 0.9
wal_buffers = 16MB
default_statistics_target = 100
random_page_cost = 1.1
effective_io_concurrency = 200
```

#### Backup and Maintenance

```bash
# Regular maintenance
pg_dump -U ifthisthenat ifthisthenat > backup.sql

# Vacuum and analyze
VACUUM ANALYZE;

# Check table sizes
SELECT schemaname, tablename, pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename))
FROM pg_tables WHERE schemaname = 'public';
```

## Queue Adapters

ifthisthenat supports multiple queue adapters for different deployment scenarios.

### Blueprint Queue Adapters

#### MPSC (In-Memory) Adapter

Best for single-instance deployments.

**Configuration:**
```bash
BLUEPRINT_QUEUE_ADAPTER=mpsc
BLUEPRINT_QUEUE_BUFFER_SIZE=5000    # Default: 5000
```

**Characteristics:**
- **Latency**: < 1ms
- **Throughput**: 100k+ items/sec
- **Memory**: O(queue_size)
- **Persistence**: None
- **Distribution**: Single instance only

**Use Cases:**
- Development and testing
- Single-node production deployments
- Low-latency requirements
- Simple deployment scenarios

```rust
// Implementation example
let adapter = Arc::new(MpscBlueprintQueueAdapter::new(5000));
let task = BlueprintEvaluationTask::with_adapter(
    factory, storage, nodes, adapter, cancel_token
);
```

#### Redis Queue Adapter (Future)

For distributed processing across multiple instances.

**Planned Configuration:**
```bash
BLUEPRINT_QUEUE_ADAPTER=redis
REDIS_URL=redis://redis:6379
BLUEPRINT_QUEUE_REDIS_PREFIX=queue:blueprint:
BLUEPRINT_QUEUE_REDIS_WORKER_ID=worker-001
```

**Characteristics:**
- **Latency**: 1-5ms
- **Throughput**: 10k-50k items/sec
- **Memory**: Configurable
- **Persistence**: Configurable (RDB/AOF)
- **Distribution**: Multiple instances

**Benefits:**
- Persistence across restarts
- Distributed processing
- Built-in TTL support
- Pub/sub for notifications

#### PostgreSQL Queue Adapter (Future)

For transactional guarantees and audit trails.

**Planned Configuration:**
```bash
BLUEPRINT_QUEUE_ADAPTER=postgres
# Uses existing DATABASE_URL
BLUEPRINT_QUEUE_TABLE=blueprint_queue
```

**Characteristics:**
- **Latency**: 5-20ms
- **Throughput**: 1k-10k items/sec
- **Memory**: Minimal
- **Persistence**: Full ACID
- **Distribution**: Multiple instances with coordination

**Benefits:**
- ACID compliance
- Integration with existing database
- Query capabilities
- Audit trail

### Webhook Queue Adapters

The webhook system supports both direct delivery and queued processing.

#### Direct Evaluator

Synchronous webhook delivery.

**Configuration:**
```bash
WEBHOOK_QUEUE_ENABLED=false
```

**Characteristics:**
- **Mode**: Synchronous
- **Retry Logic**: None
- **Feedback**: Immediate
- **Blocking**: Yes (blocks blueprint pipeline)

**Use Cases:**
- Low-volume scenarios (< 10 webhooks/sec)
- When immediate response is needed
- Testing and development

#### Queue Evaluator

Asynchronous webhook processing with retry logic.

**Configuration:**
```bash
WEBHOOK_QUEUE_ENABLED=true
WEBHOOK_MAX_CONCURRENT=10
WEBHOOK_QUEUE_SIZE=1000
WEBHOOK_MAX_RETRIES=3
WEBHOOK_RETRY_DELAY_MS=1000
```

**Characteristics:**
- **Mode**: Asynchronous
- **Retry Logic**: Exponential backoff
- **Feedback**: Queue status only
- **Blocking**: No

**Use Cases:**
- High-volume scenarios (> 10 webhooks/sec)
- Reliability requirements
- Production environments

## Cursor Storage

Cursor storage tracks the Jetstream consumption position to enable resumption after restarts.

### Redis Cursor Storage

**Configuration:**
```bash
REDIS_URL=redis://localhost:6379
REDIS_CURSOR_KEY=ifthisthenat:jetstream:cursor    # Default
REDIS_CURSOR_TTL_SECONDS=86400                    # Default: 24 hours
```

**Benefits:**
- Distributed access across multiple instances
- Atomic operations
- Automatic expiration
- Better performance than file I/O
- Fallback support

**How it Works:**
1. Receives Jetstream events with `time_us` timestamps
2. Periodically writes latest timestamp to Redis (every 5 seconds)
3. Sets TTL on the key to prevent stale data

### File-based Cursor Storage

**Configuration:**
```bash
JETSTREAM_CURSOR_PATH=/app/data/cursor.json
```

**Benefits:**
- Simple setup
- No external dependencies
- Persistent across container restarts
- Local file system performance

**Use Cases:**
- Single-instance deployments
- Development environments
- When Redis is not available

### Priority Order

The system uses this priority order for cursor storage:
1. **Redis** (if `REDIS_URL` is configured and accessible)
2. **File** (if `JETSTREAM_CURSOR_PATH` is configured)
3. **No persistence** (if neither is configured)

## Evaluation Result Storage

### Tracing Storage (Default)

Logs evaluation results to the tracing system.

**Configuration:**
```bash
# No additional configuration needed
# Results appear in application logs
```

**Characteristics:**
- Integrated with application logging
- No additional storage requirements
- Suitable for development and basic monitoring

### Filesystem Storage

Stores evaluation results as JSON files for audit trails.

**Configuration:**
```bash
EVALUATION_LOG_DIRECTORY=/var/log/iftta/logs
```

**Directory Structure:**
```
/var/log/iftta/logs/
├── {did}/
│   └── {blueprint_id}/
│       ├── {evaluation_id_1}.json
│       ├── {evaluation_id_2}.json
│       └── {evaluation_id_3}.json
```

**JSON File Format:**
```json
{
  "id": "01K51CKSQ82X3GVD576N0KB5Y0",
  "aturi": "at://did:plc:user/tools.graze.ifthisthenat.blueprint/example",
  "queued_at": "2024-01-15T10:30:00.123Z",
  "completed_at": "2024-01-15T10:30:00.456Z",
  "result": "OK"
}
```

**Features:**
- Automatic directory creation
- Efficient retrieval by ID or blueprint
- Sorting by completion time
- Human-readable audit logs

**Maintenance:**
```bash
# Log rotation
/var/log/iftta/logs/*/*/*/*.json {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
}

# Disk usage monitoring
du -sh /var/log/iftta/logs
find /var/log/iftta/logs -name "*.json" -mtime +30 -delete
```

## Denylist Storage

### PostgreSQL Denylist Storage

Uses PostgreSQL with bloom filter optimization for efficient membership checking.

**Configuration:**
```bash
# Automatic when PostgreSQL is available
# Optional bloom filter tuning:
DENYLIST_EXPECTED_ITEMS=100000    # Default: 100,000
DENYLIST_FALSE_POSITIVE_RATE=0.01 # Default: 0.01 (1%)
```

**Architecture:**
```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│  Application    │───▶│  DenyListManager │───▶│  BloomFilter    │
│  Code           │    │                  │    │  (Fast Check)   │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                │                        │
                                ▼                        ▼
                       ┌──────────────────┐    ┌─────────────────┐
                       │  DenyListStorage │───▶│  PostgreSQL     │
                       │                  │    │  (Persistent)   │
                       └──────────────────┘    └─────────────────┘
```

**Performance:**
- **Fast negative responses**: Bloom filter provides O(1) lookup
- **Memory efficient**: Uses minimal memory regardless of denylist size
- **Thread-safe**: Concurrent reads are very fast
- **False positives**: May occur but are verified against database

**Database Schema:**
```sql
CREATE TABLE denylist (
    item TEXT PRIMARY KEY,           -- The denied item
    created_at TIMESTAMPTZ NOT NULL, -- When item was added
    note TEXT                        -- Optional reason for denial
);

CREATE INDEX idx_denylist_created_at ON denylist(created_at DESC);
```

### No-op Denylist Storage

For testing or when denylist functionality is disabled.

**Configuration:**
```bash
# Automatically used when PostgreSQL is not available
# Or explicitly configured for testing
```

**Characteristics:**
- Always returns false (nothing is denied)
- No storage requirements
- Suitable for development and testing

## Webhook Task System

The webhook task system provides reliable webhook delivery with configurable backends.

### MPSC Webhook Queue

In-memory webhook queue using tokio MPSC channels.

**Configuration:**
```bash
WEBHOOK_QUEUE_ENABLED=true
WEBHOOK_QUEUE_SIZE=1000
WEBHOOK_MAX_CONCURRENT=10
```

**Characteristics:**
- **Speed**: Very fast (sub-millisecond latency)
- **Reliability**: Lost on restart
- **Scaling**: Single instance only
- **Memory**: Queue size × webhook size

### Future Webhook Queue Adapters

#### Redis Webhook Queue

**Planned Configuration:**
```bash
WEBHOOK_QUEUE_ADAPTER=redis
REDIS_URL=redis://redis:6379
WEBHOOK_QUEUE_REDIS_PREFIX=queue:webhook:
```

**Benefits:**
- Persistence across restarts
- Distributed processing
- Configurable TTL
- Multiple consumer support

#### PostgreSQL Webhook Queue

**Planned Configuration:**
```bash
WEBHOOK_QUEUE_ADAPTER=postgres
# Uses existing DATABASE_URL
WEBHOOK_QUEUE_TABLE=webhook_tasks
```

**Benefits:**
- ACID compliance
- Transactional delivery guarantees
- Rich query capabilities
- Audit trail

## Performance Comparison

### Queue Adapter Performance

| Adapter | Latency | Throughput | Memory | Persistence | Distribution |
|---------|---------|------------|---------|-------------|--------------|
| MPSC | < 1ms | 100k+/sec | High | None | Single |
| Redis | 1-5ms | 10-50k/sec | Medium | Yes | Multi |
| PostgreSQL | 5-20ms | 1-10k/sec | Low | Full | Multi |

### Storage Performance

| Storage Type | Read Latency | Write Latency | Scalability | Consistency |
|--------------|-------------|---------------|-------------|-------------|
| PostgreSQL | 1-5ms | 1-10ms | Vertical | ACID |
| Redis | < 1ms | < 1ms | Horizontal | Eventual |
| File System | < 1ms | 1-5ms | Limited | None |

### Cursor Storage Performance

| Type | Latency | Reliability | Distribution | Complexity |
|------|---------|-------------|--------------|------------|
| Redis | 1-2ms | High | Yes | Medium |
| File | < 1ms | Medium | No | Low |
| None | 0ms | None | N/A | None |

## Deployment Recommendations

### Single Instance Deployment

**Recommended Configuration:**
```bash
# Use MPSC for simplicity and performance
BLUEPRINT_QUEUE_ADAPTER=mpsc
BLUEPRINT_QUEUE_BUFFER_SIZE=5000

# Use file-based cursor for persistence
JETSTREAM_CURSOR_PATH=/app/data/cursor.json

# PostgreSQL for primary storage
DATABASE_URL=postgres://user:pass@localhost:5432/db

# Direct webhook delivery for simplicity
WEBHOOK_QUEUE_ENABLED=false
```

**Use Cases:**
- Development environments
- Small production deployments
- Cost-sensitive deployments

### Multi-Instance Deployment

**Recommended Configuration:**
```bash
# Use Redis for distributed queuing
BLUEPRINT_QUEUE_ADAPTER=redis
REDIS_URL=redis://redis:6379

# Redis cursor for shared state
REDIS_CURSOR_KEY=ifthisthenat:jetstream:cursor

# PostgreSQL with connection pooling
DATABASE_URL=postgres://user:pass@postgres:5432/db
DATABASE_MAX_CONNECTIONS=20

# Async webhook processing
WEBHOOK_QUEUE_ENABLED=true
WEBHOOK_MAX_CONCURRENT=20

# Enable leadership election
BLUEPRINT_LEADERSHIP_ENABLED=true
WEBHOOK_LEADERSHIP_ENABLED=true
SCHEDULER_LEADERSHIP_ENABLED=true
```

**Use Cases:**
- High-availability production deployments
- High-volume processing
- Kubernetes deployments

### High-Performance Deployment

**Recommended Configuration:**
```bash
# Maximize throughput
JETSTREAM_WORKER_THREADS=4
WEBHOOK_MAX_CONCURRENT=50
WEBHOOK_QUEUE_SIZE=10000

# Large caches
IDENTITY_CACHE_SIZE=100000
BLUEPRINT_QUEUE_BUFFER_SIZE=20000

# Distributed processing
JETSTREAM_TOTAL_INSTANCES=3
JETSTREAM_PARTITION_STRATEGY=did

# Redis for all caching and queuing
REDIS_URL=redis://redis-cluster:6379
BLUEPRINT_QUEUE_ADAPTER=redis
```

**Use Cases:**
- High-volume AT Protocol processing
- Real-time automation requirements
- Large-scale deployments

### Resource-Constrained Deployment

**Recommended Configuration:**
```bash
# Minimal resource usage
JETSTREAM_WORKER_THREADS=1
WEBHOOK_MAX_CONCURRENT=3
WEBHOOK_QUEUE_SIZE=100

# Small caches
IDENTITY_CACHE_SIZE=1000
BLUEPRINT_QUEUE_BUFFER_SIZE=1000

# Single instance setup
BLUEPRINT_QUEUE_ADAPTER=mpsc
JETSTREAM_CURSOR_PATH=/app/data/cursor.json

# Disable optional features
SCHEDULER_ENABLED=false
REDIS_URL=  # Not used
```

**Use Cases:**
- IoT deployments
- Edge computing
- Development environments
- Budget-constrained deployments

This guide provides the foundation for choosing appropriate storage and queue configurations based on your deployment requirements and constraints.