# ifthisthenat (IFTTA)

**If This Then AT** - A high-performance AT Protocol automation service written in Rust. Process events from Jetstream (Bluesky's firehose), webhooks, and periodic schedules using a blueprint-based system with DataLogic expressions for rule evaluation.

**Version**: 1.0.0-rc.1
**Repository**: [github.com/graze-social/iftta](https://github.com/graze-social/iftta)

## ⚠️ Release Candidate

**Version 1.0.0-rc.1** - This release candidate includes comprehensive error handling, production features, and extensive testing. While suitable for production use, thorough testing is recommended for your specific use case.

## Performance

ifthisthenat is designed for high throughput and low latency:

- **Binary serialization** reduces cache storage by ~40% compared to JSON
- **Multi-layer caching** with in-memory, Redis, and PostgreSQL support
- **Work shedding** prevents unbounded queue growth
- **Configurable TTLs** for fine-tuning cache freshness vs. performance
- **Connection pooling** for Redis and PostgreSQL minimizes overhead
- **Metrics support** via StatsD for production monitoring

Benchmarks:
- Condition node evaluation: ~30-50μs per evaluation
- Sentiment analysis: ~300-500μs per analysis
- Blueprint caching: 40% storage reduction with binary serialization
- Jetstream processing: Supports partitioning for horizontal scaling

## Features

- **Blueprint System**: Define automation workflows with ordered node evaluation
- **Entry Nodes**:
  - `jetstream_entry`: Process AT Protocol firehose events
  - `webhook_entry`: Handle external webhook triggers
  - `periodic_entry`: Cron-based scheduled tasks
  - `zap_entry`: Zapier integration support
- **Processing Nodes**:
  - `condition`: Boolean flow control with DataLogic expressions
  - `transform`: Data manipulation and transformation
  - `facet_text`: Process text with AT Protocol facets/mentions
  - `sentiment_analysis`: Analyze text sentiment
  - `parse_aturi`: Parse AT-URI strings
  - `get_record`: Retrieve AT Protocol records
- **Action Nodes**:
  - `publish_record`: Create AT Protocol records
  - `publish_webhook_direct`: Synchronous webhook delivery
  - `publish_webhook_queue`: Async webhooks with retries
  - `debug_action`: Development logging
- **Production Features**:
  - OAuth 2.0/2.1/OIDC authentication with AT Protocol
  - Multi-instance support with Redis coordination
  - Distributed leadership election
  - Comprehensive error handling with structured codes
  - Binary serialization for 40% storage reduction
  - Sentry error tracking integration
  - Health checks and graceful shutdown
- **Queue & Throttling**:
  - Multiple adapter support (MPSC, Redis)
  - Blueprint throttling (per-authority, per-AT-URI)
  - Work shedding for overload protection
  - Denylist system for blocking bad actors

## Building

### Prerequisites

- Rust 1.70 or later
- PostgreSQL 14+ (required)
- Redis 6.2+ (optional, for multi-instance deployments)
- Docker & Docker Compose (for development)

### Build Commands

```bash
# Clone the repository
git clone https://github.com/graze-social/iftta.git
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
- `ISSUER_DID`: Service DID (must be valid did:plc or did:web)
- `AIP_HOSTNAME`: AT Protocol OAuth hostname
- `AIP_CLIENT_ID`: OAuth client ID
- `AIP_CLIENT_SECRET`: OAuth client secret

### Example Minimal Setup

```bash
# Generate cookie key
export HTTP_COOKIE_KEY=$(openssl rand -base64 64)

# Run with minimal config
EXTERNAL_BASE=http://localhost:8080 \
HTTP_COOKIE_KEY=$HTTP_COOKIE_KEY \
ISSUER_DID=did:plc:test123 \
AIP_HOSTNAME=test.com \
AIP_CLIENT_ID=test \
AIP_CLIENT_SECRET=test \
DATABASE_URL=postgres://user:pass@localhost:5432/ifthisthenat \
cargo run
```

This will start ifthisthenat with:
- HTTP server on port 8080 (default)
- In-memory caching only
- MPSC queue adapter for async processing
- Jetstream consumer enabled (default)
- Scheduler enabled for periodic tasks (default)
- Noop throttling and denylist (default)

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
   - Entry nodes: `jetstream_entry`, `webhook_entry`, `periodic_entry`, `zap_entry`
   - Processing nodes: `condition`, `transform`, `facet_text`, `sentiment_analysis`, `parse_aturi`, `get_record`
   - Action nodes: `publish_record`, `publish_webhook_direct`, `publish_webhook_queue`, `debug_action`

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
   - Blueprint and node caching with multiple backends
   - Session management for OAuth
   - Evaluation result storage (filesystem or noop)

## Configuration

### Environment Variables

Configuration follows the 12-factor app methodology using environment variables exclusively.

#### Required Variables
- `EXTERNAL_BASE`: External base URL
- `HTTP_COOKIE_KEY`: Cookie encryption key (generate with `openssl rand -base64 64`)
- `ISSUER_DID`: Service DID (valid did:plc or did:web)
- `AIP_HOSTNAME`: AT Protocol OAuth hostname
- `AIP_CLIENT_ID`: OAuth client ID
- `AIP_CLIENT_SECRET`: OAuth client secret

#### Core Service (Optional)
- `HTTP_PORT`: Server port (default: 8080)
- `HTTP_STATIC_PATH`: Static file path (default: `static`)
- `HTTP_CLIENT_TIMEOUT`: Client timeout in seconds (default: 8)
- `USER_AGENT`: Custom user agent (default: `ifthisthenat/VERSION`)
- `DATABASE_URL`: PostgreSQL URL (default: `postgres://username:password@localhost:5432/ifthisthenat`)

#### OAuth/Authentication
- `AIP_OAUTH_SCOPE`: OAuth scope (default: `openid email profile atproto account:email`)
- `ADMIN_DIDS`: Semicolon-separated admin DIDs
- `ALLOWED_IDENTITIES`: Comma-separated allowed DIDs

#### Jetstream Configuration
- `JETSTREAM_ENABLED`: Enable Jetstream consumer (default: true)
- `JETSTREAM_COLLECTIONS`: Comma-separated collections to monitor
- `JETSTREAM_WORKER_THREADS`: Number of worker threads (default: 1)
- `JETSTREAM_PARTITION`: Partition config (format: `INDEX:TOTAL`)
- `JETSTREAM_PARTITION_STRATEGY`: Partitioning strategy (default: `metrohash`)
- `JETSTREAM_CURSOR_PATH`: File path for cursor persistence

#### Webhook Queue
- `WEBHOOK_QUEUE_ENABLED`: Enable webhook queue (default: false)
- `WEBHOOK_MAX_CONCURRENT`: Max concurrent webhooks (default: 10)
- `WEBHOOK_DEFAULT_TIMEOUT_MS`: Default timeout in ms (default: 30000)
- `WEBHOOK_MAX_RETRIES`: Max retry attempts (default: 3)
- `WEBHOOK_RETRY_DELAY_MS`: Delay between retries (default: 1000)
- `WEBHOOK_LOG_BODIES`: Log webhook bodies (default: false)

#### Blueprint Processing
- `BLUEPRINT_QUEUE_ADAPTER`: Queue adapter type (`mpsc` or `redis`, default: `mpsc`)
- `BLUEPRINT_QUEUE_BUFFER_SIZE`: MPSC buffer size (default: 10000)
- `BLUEPRINT_QUEUE_MAX_RETRIES`: Max retries (default: 3)
- `BLUEPRINT_THROTTLER`: Throttler type (`noop`, `redis`, `redis-per-identity`, default: `noop`)
- `BLUEPRINT_THROTTLER_AUTHORITY_LIMIT`: Authority rate limit
- `BLUEPRINT_THROTTLER_AUTHORITY_WINDOW`: Authority window in seconds
- `BLUEPRINT_THROTTLER_ATURI_LIMIT`: AT-URI rate limit
- `BLUEPRINT_THROTTLER_ATURI_WINDOW`: AT-URI window in seconds

#### Scheduler
- `SCHEDULER_ENABLED`: Enable periodic scheduler (default: true)
- `SCHEDULER_CHECK_INTERVAL_SECS`: Check interval (default: 60)
- `SCHEDULER_CACHE_RELOAD_SECS`: Cache reload interval (default: 300)
- `SCHEDULER_MAX_CONCURRENT`: Max concurrent evaluations (default: 10)

#### Monitoring
- `SENTRY_DSN`: Sentry DSN for error tracking
- `SENTRY_ENVIRONMENT`: Environment name (default: `development`)
- `SENTRY_TRACES_SAMPLE_RATE`: Trace sampling rate (0.0-1.0, default: 0.01)
- `SENTRY_DEBUG`: Enable Sentry debug mode (default: false)

#### Advanced
- `REDIS_URL`: Redis connection URL (enables advanced features)
- `REDIS_CURSOR_TTL_SECONDS`: TTL for Redis cursors (default: 86400)
- `DENYLIST_TYPE`: Denylist type (`noop` or `postgres`, default: `noop`)
- `DISABLED_NODE_TYPES`: Comma-separated node types to disable
- `EVALUATION_STORAGE_TYPE`: Storage type (`noop` or `filesystem`, default: `noop`)
- `EVALUATION_STORAGE_DIRECTORY`: Directory for filesystem storage

### Production Configuration Examples

#### Single-Instance Production

```bash
EXTERNAL_BASE=https://iftta.example.com \
HTTP_PORT=3000 \
HTTP_COOKIE_KEY=$COOKIE_KEY \
ISSUER_DID=did:plc:yourservice \
AIP_HOSTNAME=your-oauth.com \
AIP_CLIENT_ID=your-client-id \
AIP_CLIENT_SECRET=your-client-secret \
DATABASE_URL=postgres://user:pass@localhost/iftta \
JETSTREAM_ENABLED=true \
WEBHOOK_QUEUE_ENABLED=true \
DENYLIST_TYPE=postgres \
SENTRY_DSN=your-sentry-dsn \
SENTRY_ENVIRONMENT=production \
RUST_LOG=info \
./target/release/ifthisthenat
```

#### Multi-Instance with Redis (High Availability)

```bash
EXTERNAL_BASE=https://iftta.example.com \
HTTP_PORT=3000 \
HTTP_COOKIE_KEY=$COOKIE_KEY \
ISSUER_DID=did:plc:yourservice \
AIP_HOSTNAME=your-oauth.com \
AIP_CLIENT_ID=your-client-id \
AIP_CLIENT_SECRET=your-client-secret \
DATABASE_URL=postgres://user:pass@pghost/iftta \
REDIS_URL=redis://redishost:6379 \
JETSTREAM_ENABLED=true \
JETSTREAM_WORKER_THREADS=4 \
JETSTREAM_PARTITION=0:3 \
BLUEPRINT_QUEUE_ADAPTER=redis \
BLUEPRINT_THROTTLER=redis-per-identity \
WEBHOOK_QUEUE_ENABLED=true \
DENYLIST_TYPE=postgres \
SENTRY_DSN=your-sentry-dsn \
SENTRY_ENVIRONMENT=production \
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

// Hash functions for unique IDs
{"metrohash": ["string1", "string2"]}

// Parse AT-URI components
{"parse_aturi": ["at://did:plc:example/app.bsky.feed.post/abc"]}

// Process text with facets/mentions
{"facet_text": ["Hello @user.bsky.social!"]}
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
- [Contributing Guide](CONTRIBUTING.md) - How to contribute
- [Changelog](CHANGELOG.md) - Release history and changes

## Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) for details on our code of conduct and the process for submitting pull requests.

## License

This project is licensed under the MIT License. Copyright © 2025 Nick Gerakines and Graze Social. See the [LICENSE](LICENSE) file for details.

## Acknowledgments

- AT Protocol team for the protocol and specifications
- Bluesky team for Jetstream and the firehose
- Rust community for excellent libraries and tooling
- Contributors and early adopters for feedback and improvements