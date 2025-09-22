# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ifthisthenat is an AT Protocol automation service built in Rust that processes events from Jetstream (Bluesky's firehose), webhooks, and periodic schedules. It uses a blueprint-based system with DataLogic expressions for rule evaluation and supports various actions like publishing records and sending webhooks.

**Version**: 1.0.0-rc.1
**GitHub**: https://github.com/graze-social/iftta

## Quick Start

```bash
# Clone repository
git clone https://github.com/graze-social/iftta
cd ifthisthenat

# Set up environment
cp .env.example .env  # Edit with your configuration
docker-compose up -d  # Start PostgreSQL
sqlx migrate run      # Run migrations

# Configure minimum required variables
export EXTERNAL_BASE=http://localhost:8080
export HTTP_COOKIE_KEY=$(openssl rand -base64 64)
export ISSUER_DID=did:plc:test
export AIP_HOSTNAME=test.com
export AIP_CLIENT_ID=test
export AIP_CLIENT_SECRET=test

# Run the service
cargo run
```

## Development Commands

### Build & Run
```bash
cargo build                  # Development build
cargo build --release        # Production build
cargo run                    # Run in development mode
cargo run --release          # Run in production mode
./target/debug/ifthisthenat --version  # Check version
```

### Testing & Benchmarks
```bash
cargo test                   # Run all tests
cargo test test_name         # Run specific test by name
cargo test module::tests     # Run tests in specific module
cargo test --lib             # Run library tests only
cargo check                  # Quick compilation check
cargo bench                  # Run performance benchmarks

# Test with specific features
cargo test --features redis  # Test with Redis support
cargo test -- --nocapture    # Show test output
cargo test -- --test-threads=1  # Run tests serially

# Run specific test categories
cargo test engine::          # Test engine components
cargo test storage::         # Test storage layer
cargo test tasks::          # Test background tasks
```

### Writing Tests
- Unit tests go in the same file as the code (in `mod tests`)
- Integration tests go in `tests/` directory
- Use `#[tokio::test]` for async tests
- Mock external dependencies using traits
- Test both success and error cases

### Database
```bash
docker-compose up -d         # Start development PostgreSQL
sqlx migrate run            # Run database migrations
```

### Development Workflow
```bash
# Start development environment
docker-compose up -d         # Start PostgreSQL and Redis
sqlx migrate run            # Apply database migrations
cargo run                   # Run development server

# Common development tasks
cargo fmt                   # Format code
cargo clippy               # Run linter
cargo test --lib          # Run unit tests
cargo test -- --nocapture # Run tests with output

# Debug specific components
RUST_LOG=debug cargo run   # Enable debug logging
RUST_LOG=ifthisthenat=trace cargo run  # Trace level for app only
SCHEDULER_DEBUG_LOGGING=true cargo run  # Debug scheduler
WEBHOOK_LOG_BODIES=true cargo run      # Log webhook bodies
```

## Architecture

### Core Components

1. **Blueprint System** (`src/storage/blueprint.rs`, `src/processor.rs`)
   - Blueprints define automation workflows with ordered node evaluation
   - Each blueprint has an AT-URI identifier and evaluation order
   - Blueprints are cached in memory/Redis for performance

2. **Node Types** (`src/engine/`)

   **Entry Nodes** (must be first in blueprint):
   - `jetstream_entry`: Filters Jetstream events from firehose
   - `webhook_entry`: Filters incoming webhook requests
   - `periodic_entry`: Triggered by cron schedules
   - `zap_entry`: Zapier integration entry point

   **Processing Nodes**:
   - `condition`: Boolean flow control using DataLogic expressions
   - `transform`: Data transformation using DataLogic expressions
   - `facet_text`: Processes text with facets/mentions for AT Protocol
   - `sentiment_analysis`: Analyzes text sentiment
   - `parse_aturi`: Parses AT-URI strings into components
   - `get_record`: Retrieves AT Protocol records

   **Action Nodes**:
   - `publish_record`: Creates AT Protocol records
   - `publish_webhook_direct`: Sends webhooks directly (synchronous)
   - `publish_webhook_queue`: Sends webhooks via queue (async with retries)
   - `debug_action`: Logs data for debugging

3. **Evaluation Engine** (`src/engine/evaluator.rs`)
   - Uses `datalogic-rs` for expression evaluation
   - Sequential node processing with early termination on failure
   - Async evaluation via queue adapters (memory/Redis)

4. **Consumer System** (`src/consumer.rs`)
   - Handles Jetstream events with partitioning support
   - Multi-worker thread processing
   - Redis or file-based cursor persistence

5. **Task System** (`src/tasks/`)
   - `BlueprintEvaluationTask`: Processes queued blueprints
   - `WebhookTask`: Manages webhook delivery with retries
   - `SchedulerTask`: Handles periodic entry nodes
   - `BlueprintQueueAdapter`: Queue adapter for blueprint work distribution

6. **Storage Layer** (`src/storage/`)
   - PostgreSQL for persistent data
   - Redis for caching and queues (optional)
   - Memory caches with LRU eviction
   - Blueprint and node caching with multiple backends
   - Session management for OAuth
   - Evaluation result storage (filesystem or noop)

7. **Identity & Access** (`src/identity_cache.rs`, `src/storage/identity.rs`)
   - DID resolution and caching
   - Handle resolution via quickdid
   - PLC directory integration
   - Admin and allowed identity management

8. **Leadership & Coordination** (`src/leadership.rs`)
   - Distributed leadership election for multi-instance deployments
   - Redis-based coordination
   - Automatic failover support

9. **DenyList System** (`src/denylist/`)
   - Flexible denylist for blocking specific DIDs or AT-URIs
   - PostgreSQL or noop backends
   - Real-time blocking without restarts

## Key Configuration

### Required Environment Variables
- `EXTERNAL_BASE`: Service base URL (e.g., `https://iftta.example.com`)
- `HTTP_COOKIE_KEY`: Cookie encryption key (generate with `openssl rand -base64 64`)
- `ISSUER_DID`: Service DID (must be valid did:plc or did:web)
- `AIP_HOSTNAME`: AT Protocol OAuth hostname
- `AIP_CLIENT_ID`: OAuth client ID
- `AIP_CLIENT_SECRET`: OAuth client secret

### Optional Environment Variables

**HTTP & Server**:
- `HTTP_PORT`: Server port (default: 8080)
- `HTTP_STATIC_PATH`: Static file path (default: `static`)
- `HTTP_CLIENT_TIMEOUT`: Client timeout in seconds (default: 8)
- `USER_AGENT`: Custom user agent (default: `ifthisthenat/VERSION`)

**Database**:
- `DATABASE_URL`: PostgreSQL URL (default: `postgres://username:password@localhost:5432/ifthisthenat`)
- `REDIS_URL`: Redis connection URL (optional, enables advanced features)
- `REDIS_CURSOR_TTL_SECONDS`: TTL for Redis cursors (default: 86400)

**Jetstream Consumer**:
- `JETSTREAM_ENABLED`: Enable Jetstream consumer (default: true)
- `JETSTREAM_COLLECTIONS`: Comma-separated collections to monitor
- `JETSTREAM_WORKER_THREADS`: Number of worker threads (default: 1)
- `JETSTREAM_PARTITION`: Partition config (format: `INDEX:TOTAL`)
- `JETSTREAM_PARTITION_STRATEGY`: Strategy for partitioning (default: `metrohash`)
- `JETSTREAM_CURSOR_PATH`: File path for cursor persistence

**Webhook Queue**:
- `WEBHOOK_QUEUE_ENABLED`: Enable webhook queue (default: false)
- `WEBHOOK_MAX_CONCURRENT`: Max concurrent webhooks (default: 10)
- `WEBHOOK_DEFAULT_TIMEOUT_MS`: Default timeout in ms (default: 30000)
- `WEBHOOK_MAX_RETRIES`: Max retry attempts (default: 3)
- `WEBHOOK_RETRY_DELAY_MS`: Delay between retries (default: 1000)
- `WEBHOOK_LOG_BODIES`: Log webhook bodies (default: false)

**Blueprint Queue**:
- `BLUEPRINT_QUEUE_ADAPTER`: Queue adapter type (`mpsc` or `redis`, default: `mpsc`)
- `BLUEPRINT_QUEUE_BUFFER_SIZE`: MPSC buffer size (default: 10000)
- `BLUEPRINT_QUEUE_MAX_RETRIES`: Max retries (default: 3)

**Scheduler**:
- `SCHEDULER_ENABLED`: Enable periodic scheduler (default: true)
- `SCHEDULER_CHECK_INTERVAL_SECS`: Check interval (default: 60)
- `SCHEDULER_CACHE_RELOAD_SECS`: Cache reload interval (default: 300)
- `SCHEDULER_MAX_CONCURRENT`: Max concurrent evaluations (default: 10)

**Throttling**:
- `BLUEPRINT_THROTTLER`: Throttler type (`noop`, `redis`, `redis-per-identity`, default: `noop`)
- `BLUEPRINT_THROTTLER_AUTHORITY_LIMIT`: Authority rate limit
- `BLUEPRINT_THROTTLER_AUTHORITY_WINDOW`: Authority window in seconds
- `BLUEPRINT_THROTTLER_ATURI_LIMIT`: AT-URI rate limit
- `BLUEPRINT_THROTTLER_ATURI_WINDOW`: AT-URI window in seconds

**Security & Access Control**:
- `ADMIN_DIDS`: Semicolon-separated admin DIDs
- `ALLOWED_IDENTITIES`: Comma-separated allowed DIDs
- `DENYLIST_TYPE`: Denylist type (`noop` or `postgres`, default: `noop`)
- `DISABLED_NODE_TYPES`: Comma-separated node types to disable

**Monitoring**:
- `SENTRY_DSN`: Sentry DSN for error tracking
- `SENTRY_ENVIRONMENT`: Environment name (default: `development`)
- `SENTRY_TRACES_SAMPLE_RATE`: Trace sampling rate (0.0-1.0, default: 0.01)
- `SENTRY_DEBUG`: Enable Sentry debug mode (default: false)

**Evaluation Storage**:
- `EVALUATION_STORAGE_TYPE`: Storage type (`noop` or `filesystem`, default: `noop`)
- `EVALUATION_STORAGE_DIRECTORY`: Directory for filesystem storage

**OAuth**:
- `AIP_OAUTH_SCOPE`: OAuth scope (default: `openid email profile atproto account:email`)

## API Endpoints

XRPC endpoints for blueprint management:
- `GET /xrpc/tools.graze.ifthisthenat.getBlueprints`
- `GET /xrpc/tools.graze.ifthisthenat.getBlueprint`
- `POST /xrpc/tools.graze.ifthisthenat.updateBlueprint`
- `POST /xrpc/tools.graze.ifthisthenat.deleteBlueprint`

See `API.md` for detailed blueprint structure and node payload examples.

## DataLogic Expression System

The engine uses DataLogic for dynamic evaluation.

### Common Patterns
- Value extraction: `{"val": ["path", "to", "field"]}`
- Comparisons: `{"==": [{"val": ["field"]}, "value"]}`
- String operations: `{"cat": ["prefix", {"val": ["field"]}, "suffix"]}`
- Time functions: `{"now": []}`, `{"datetime": [timestamp]}`
- Hash functions: `{"metrohash": ["string1", "string2"]}`
- AT-URI parsing: `{"parse_aturi": ["at://did:plc:example/app.bsky.feed.post/abc"]}`
- Facet text: `{"facet_text": ["Hello @user.bsky.social!"]}`

### Advanced Examples
```json
// Check if post contains specific hashtag
{"contains": [{"val": ["tags"]}, "rust"]}

// Complex conditional
{"and": [
  {">=": [{"val": ["metrics", "likes"]}, 10]},
  {"contains": [{"val": ["text"]}, "AT Protocol"]}
]}

// Transform AT-URI to get collection
{"val": [{"parse_aturi": [{"val": ["uri"]}]}, "collection"]}

// Generate unique ID from multiple fields
{"metrohash": [
  {"val": ["author"]},
  {"val": ["timestamp"]},
  {"val": ["collection"]}
]}
```

## Performance Considerations

- Blueprint cache reloads are rate-limited
- Use Redis for production deployments with multiple instances
- Jetstream partitioning supports horizontal scaling
- Webhook queue prevents blocking on slow endpoints
- Memory caches use LRU eviction to limit memory usage
- Connection pooling for database and Redis connections

## Production Deployment

### Recommended Configuration
```bash
# Required
EXTERNAL_BASE=https://your-domain.com
HTTP_COOKIE_KEY=$(openssl rand -base64 64)
ISSUER_DID=did:plc:yourdid
AIP_HOSTNAME=your-oauth-server.com
AIP_CLIENT_ID=your-client-id
AIP_CLIENT_SECRET=your-client-secret

# Highly Recommended for Production
REDIS_URL=redis://localhost:6379
DATABASE_URL=postgres://user:pass@db:5432/ifthisthenat
BLUEPRINT_QUEUE_ADAPTER=redis
WEBHOOK_QUEUE_ENABLED=true
DENYLIST_TYPE=postgres
BLUEPRINT_THROTTLER=redis-per-identity

# Monitoring
SENTRY_DSN=your-sentry-dsn
SENTRY_ENVIRONMENT=production
```

### Scaling Considerations
- Use Jetstream partitioning for multiple instances
- Enable Redis for distributed caching and queues
- Configure blueprint throttling to prevent abuse
- Set appropriate worker thread counts based on CPU cores
- Monitor memory usage and adjust cache sizes

### Security Best Practices
- Always set strong `HTTP_COOKIE_KEY`
- Use `ALLOWED_IDENTITIES` to restrict access
- Configure `ADMIN_DIDS` for administrative access
- Enable denylist for blocking bad actors
- Use HTTPS for `EXTERNAL_BASE`
- Rotate secrets regularly

## Error Handling

### Error Format
All error strings must use this format:

    error-iftta-<domain>-<number> <message>: <details>

Example errors:
- `error-iftta-config-1 Required environment variable not set: DATABASE_URL`
- `error-iftta-engine-2 DataLogic expression evaluation failed: {"val": ["foo"]}: Field not found`
- `error-iftta-storage-200 Database connection failed: connection refused`

### Error Organization

**Centralized Errors** (`src/errors.rs`):
- `ConfigError`: Configuration and environment variable errors
- `ConsumerError`: Jetstream consumer errors
- `EngineError`: Node evaluation and DataLogic errors
- `TaskError`: Background task errors
- `ProcessorError`: Blueprint processor errors
- `QueueError`: Queue adapter errors
- `SerializationError`: JSON and binary serialization errors
- `ValidationError`: Blueprint and node validation errors
- `AuthError`: Authentication and OAuth errors
- `IdentityError`: DID and identity resolution errors
- `LeadershipError`: Distributed leadership errors
- `HttpError`: HTTP request and response errors
- `StorageError`: Database and storage errors

**Module-Specific Errors**:
- `MetricsError` (`src/metrics.rs`): Metrics collection errors
- `BlueprintEvaluatorError` (`src/engine/blueprint_evaluator.rs`): Blueprint evaluation errors
- `SchedulerError` (`src/tasks/scheduler.rs`): Scheduler task errors
- `WebhookError` (`src/tasks/webhook.rs`): Webhook delivery errors
- `WebError` (`src/http/errors.rs`): Web handler errors

### Error Guidelines
- Use `thiserror` derive macros for all error enums
- Include contextual information in error details
- Avoid creating new errors with `anyhow!(...)`
- Remove unused error variants to keep code clean
- Prefer specific error types over generic ones