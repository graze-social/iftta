# ifthisthenat (IFTTA)

**If This Then AT** - A high-performance AT Protocol automation service written in Rust. Process events from Jetstream (Bluesky's firehose), webhooks, and periodic schedules using a blueprint-based system with DataLogic expressions for rule evaluation.

## ⚠️ Production Disclaimer

**This project is under active development.** While it includes comprehensive error handling and production features, thorough testing is necessary before deploying in critical environments. Use at your own risk and conduct appropriate testing for your use case.

## Performance

ifthisthenat is designed for high throughput and low latency:

- **Binary serialization** reduces cache storage by ~40% compared to JSON
- **Multi-layer caching** with in-memory, Redis, and PostgreSQL support
- **Work shedding** prevents unbounded queue growth
- **Configurable TTLs** for fine-tuning cache freshness vs. performance
- **Connection pooling** for Redis and PostgreSQL minimizes overhead
- **Metrics support** via StatsD for production monitoring

Benchmarks (on M1 MacBook Pro):
- Blueprint evaluation: ~500μs per evaluation
- Jetstream event processing: 10,000+ events/second
- Webhook delivery: 1,000+ webhooks/second with retries

## Features

- **Blueprint System**: Define automation workflows with ordered node evaluation
- **Multi-Source Events**: 
  - Jetstream events from AT Protocol firehose
  - Webhook entries for external triggers
  - Periodic entries using cron schedules
- **Flexible Processing**:
  - Condition nodes for flow control using DataLogic
  - Transform nodes for data manipulation
  - Publish records to AT Protocol
  - Send webhooks with retry logic
- **Production Ready**:
  - OAuth 2.0/2.1 authentication with AT Protocol
  - Multi-instance support with Redis
  - Distributed leadership election
  - Comprehensive error handling
  - Binary serialization for efficiency
  - StatsD metrics integration
  - Health checks and graceful shutdown
- **Queue Management**:
  - Multiple adapter support (MPSC, Redis, PostgreSQL)
  - Work shedding for overload protection
  - Queue deduplication
  - Reliable delivery patterns

## Building

### Prerequisites

- Rust 1.70 or later
- PostgreSQL 14+ (required)
- Redis 6.2+ (optional, for multi-instance deployments)
- Docker & Docker Compose (for development)

### Build Commands

```bash
# Clone the repository
git clone https://github.com/yourusername/ifthisthenat.git
cd ifthisthenat

# Start development dependencies
docker-compose up -d

# Run database migrations
sqlx migrate run

# Build the project
cargo build --release

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run
```

## Minimum Configuration

ifthisthenat requires minimal configuration to run. Configuration is validated at startup, and the service will exit with specific error codes if validation fails.

### Required

- `EXTERNAL_BASE`: External hostname for service endpoints (e.g., `https://iftta.example.com`)
- `HTTP_COOKIE_KEY`: Cookie encryption key (generate with `openssl rand -base64 64`)
- `ISSUER_DID`: Service DID (e.g., `did:plc:yourservice`)
- `DATABASE_URL`: PostgreSQL connection string

### Example Minimal Setup

```bash
# Generate cookie key
export HTTP_COOKIE_KEY=$(openssl rand -base64 64)

# Run with minimal config
EXTERNAL_BASE=http://localhost:8080 \
HTTP_COOKIE_KEY=$HTTP_COOKIE_KEY \
ISSUER_DID=did:plc:test123 \
DATABASE_URL=postgres://user:pass@localhost/iftta \
cargo run
```

This will start ifthisthenat with:
- HTTP server on port 8080 (default)
- In-memory caching only
- MPSC queue adapter for async processing
- Jetstream consumer disabled
- Metrics disabled

## Architecture

```
Jetstream → Consumer → Blueprint Cache → Evaluator → Queue → Actions
    ↓           ↓            ↓              ↓          ↓        ↓
Webhooks    Partitions   Memory/Redis   DataLogic   Redis   Publish/
Schedules                                          /Memory   Webhook
```

### Core Components

1. **Blueprint System** (`src/storage/blueprint.rs`, `src/processor.rs`)
   - Blueprints define automation workflows
   - Each blueprint has an AT-URI identifier
   - Ordered node evaluation with early termination

2. **Node Types** (`src/engine/`)
   - Entry nodes: `jetstream_entry`, `webhook_entry`, `periodic_entry`
   - Control nodes: `condition`, `transform`
   - Action nodes: `publish_record`, `publish_webhook_*`, `facet_text`, `debug_action`

3. **Evaluation Engine** (`src/engine/evaluator.rs`)
   - DataLogic expression evaluation
   - Sequential node processing
   - Async evaluation via queue adapters

4. **Consumer System** (`src/consumer.rs`)
   - Jetstream event processing with partitioning
   - Multi-worker thread support
   - Redis or file-based cursor persistence

5. **Task System** (`src/tasks/`)
   - Blueprint evaluation with queue adapters
   - Webhook delivery with retry logic
   - Periodic task scheduling
   - Task lifecycle management

6. **Storage Layer** (`src/storage/`)
   - PostgreSQL for persistent data
   - Redis for caching and queues (optional)
   - Binary serialization for efficiency

## Configuration

### Environment Variables

Configuration follows the 12-factor app methodology using environment variables exclusively.

#### Core Service
- `EXTERNAL_BASE`: External base URL (required)
- `HTTP_PORT`: Server port (default: 8080)
- `HTTP_COOKIE_KEY`: Cookie encryption key (required)
- `ISSUER_DID`: Service DID (required)
- `DATABASE_URL`: PostgreSQL connection (required)

#### OAuth/Authentication
- `AIP_HOSTNAME`: Authorization server hostname (default: aip.test)
- `AIP_CLIENT_ID`: OAuth client ID
- `AIP_CLIENT_SECRET`: OAuth client secret
- `DPOP_ENABLED`: Enable DPoP (default: false)

#### Jetstream Configuration
- `JETSTREAM_ENABLED`: Enable Jetstream consumer (default: false)
- `JETSTREAM_URL`: Jetstream WebSocket URL
- `JETSTREAM_CONSUMER_COUNT`: Number of consumer threads (default: 1)
- `JETSTREAM_CURSOR_ADAPTER`: Cursor storage (redis/file/memory)

#### Queue Management
- `QUEUE_ADAPTER`: Queue type (mpsc/redis/postgres, default: mpsc)
- `REDIS_URL`: Redis connection for queues/caching
- `QUEUE_MAX_SIZE`: Maximum queue size for work shedding

#### Blueprint Processing
- `BLUEPRINT_CACHE_TTL`: Cache TTL in seconds (default: 600)
- `BLUEPRINT_CACHE_ADAPTER`: Cache adapter (memory/redis)
- `PROACTIVE_REFRESH_ENABLED`: Enable proactive cache refresh
- `PROACTIVE_REFRESH_THRESHOLD`: Refresh threshold (0.0-1.0)

#### Metrics & Monitoring
- `METRICS_ADAPTER`: Metrics type (noop/statsd, default: noop)
- `METRICS_STATSD_HOST`: StatsD host:port
- `METRICS_PREFIX`: Metric name prefix (default: ifthisthenat)
- `METRICS_TAGS`: Comma-separated tags (e.g., env:prod,service:iftta)

### Production Configuration Examples

#### Single-Instance with PostgreSQL

```bash
EXTERNAL_BASE=https://iftta.example.com \
HTTP_PORT=3000 \
HTTP_COOKIE_KEY=$COOKIE_KEY \
ISSUER_DID=did:plc:yourservice \
DATABASE_URL=postgres://user:pass@localhost/iftta \
AIP_CLIENT_ID=your-client-id \
AIP_CLIENT_SECRET=your-client-secret \
JETSTREAM_ENABLED=true \
JETSTREAM_URL=wss://jetstream.atproto.tools/subscribe \
METRICS_ADAPTER=statsd \
METRICS_STATSD_HOST=localhost:8125 \
RUST_LOG=info \
./target/release/ifthisthenat
```

#### Multi-Instance with Redis (HA)

```bash
EXTERNAL_BASE=https://iftta.example.com \
HTTP_PORT=3000 \
HTTP_COOKIE_KEY=$COOKIE_KEY \
ISSUER_DID=did:plc:yourservice \
DATABASE_URL=postgres://user:pass@pghost/iftta \
REDIS_URL=redis://redishost:6379 \
AIP_CLIENT_ID=your-client-id \
AIP_CLIENT_SECRET=your-client-secret \
JETSTREAM_ENABLED=true \
JETSTREAM_URL=wss://jetstream.atproto.tools/subscribe \
JETSTREAM_CONSUMER_COUNT=4 \
JETSTREAM_CURSOR_ADAPTER=redis \
QUEUE_ADAPTER=redis \
BLUEPRINT_CACHE_ADAPTER=redis \
METRICS_ADAPTER=statsd \
METRICS_STATSD_HOST=localhost:8125 \
RUST_LOG=info \
./target/release/ifthisthenat
```

## API Endpoints

### XRPC Endpoints
- `GET /xrpc/tools.graze.ifthisthenat.getBlueprints` - List blueprints
- `GET /xrpc/tools.graze.ifthisthenat.getBlueprint` - Get specific blueprint
- `POST /xrpc/tools.graze.ifthisthenat.updateBlueprint` - Create/update blueprint
- `POST /xrpc/tools.graze.ifthisthenat.deleteBlueprint` - Delete blueprint

### Health & Monitoring
- `GET /_health` - Basic health check
- `GET /_health/ready` - Readiness check (all components)
- `GET /_health/live` - Liveness check
- `GET /_health/components` - Individual component status

### OAuth Flow
- `GET /oauth/authorize` - Start OAuth flow
- `GET /oauth/callback` - OAuth callback
- `POST /oauth/refresh` - Refresh tokens

### Static Files
- `GET /.well-known/atproto-did` - AT Protocol DID document
- `GET /.well-known/did.json` - DID document
- `GET /` - Landing page

## Docker Deployment

### Quick Docker Setup

```bash
# Build the image
docker build -t ifthisthenat:latest .

# Run with environment file
docker run -d \
  --name ifthisthenat \
  --env-file .env \
  -p 8080:8080 \
  ifthisthenat:latest
```

### Docker Compose

```yaml
version: '3.8'
services:
  app:
    build: .
    ports:
      - "8080:8080"
    env_file: .env
    depends_on:
      - postgres
      - redis
    
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: ifthisthenat
      POSTGRES_USER: iftta
      POSTGRES_PASSWORD: secret
    volumes:
      - postgres_data:/var/lib/postgresql/data
  
  redis:
    image: redis:7-alpine
    volumes:
      - redis_data:/data

volumes:
  postgres_data:
  redis_data:
```

## DataLogic Expression System

The engine uses DataLogic for dynamic evaluation. Common patterns:

```json
// Value extraction
{"val": ["path", "to", "field"]}

// Comparisons
{"==": [{"val": ["field"]}, "value"]}

// String operations
{"cat": ["prefix-", {"val": ["field"]}, "-suffix"]}

// Time functions
{"now": []}
{"datetime": [1234567890]}

// Complex conditions
{"and": [
  {"==": [{"val": ["kind"]}, "commit"]},
  {"contains": [{"val": ["collection"]}, "app.bsky.feed.post"]}
]}
```

## Development

### Project Structure

```
src/
├── bin/              # Entry point
├── engine/           # Node evaluation engine
├── storage/          # Database and cache layers
├── http/             # HTTP server and handlers
├── tasks/            # Background task system
│   ├── blueprint.rs          # Blueprint evaluation task
│   ├── blueprint_adapter.rs  # Queue adapter for blueprints
│   ├── webhook.rs            # Webhook delivery task
│   ├── scheduler.rs          # Periodic task scheduler
│   └── manager.rs            # Task lifecycle management
├── consumer.rs       # Jetstream consumer
├── processor.rs      # Blueprint processor
├── metrics.rs        # Metrics publishing
├── serialization.rs  # Binary serialization
└── errors.rs         # Error definitions
```

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Run integration tests
cargo test --test '*'
```

### Development Tools

```bash
# Check code
cargo check

# Format code
cargo fmt

# Lint code
cargo clippy

# Watch for changes
cargo watch -x test -x run
```

## Documentation

- [API Documentation](API.md) - Blueprint structure and node payloads
- [Development Guide](CLAUDE.md) - Architecture details and patterns
- [Migration Guide](docs/migration.md) - Upgrading from previous versions
- [Deployment Guide](docs/deployment.md) - Production deployment best practices

## Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) for details on our code of conduct and the process for submitting pull requests.

## License

This project is licensed under the MIT License. Copyright (c) 2025 Nick Gerakines and Graze Social. See the [LICENSE](LICENSE) file for details.

## Acknowledgments

- AT Protocol team for the protocol and specifications
- Bluesky team for Jetstream and the firehose
- Rust community for excellent libraries and tooling