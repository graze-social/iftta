# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ifthisthenat is an AT Protocol automation service built in Rust that processes events from Jetstream (Bluesky's firehose), webhooks, and periodic schedules. It uses a blueprint-based system with DataLogic expressions for rule evaluation and supports various actions like publishing records and sending webhooks.

## Development Commands

### Build & Run
```bash
cargo build                  # Development build
cargo build --release        # Production build
cargo run                    # Run in development mode
cargo run --release          # Run in production mode
./target/debug/ifthisthenat --version  # Check version
```

### Testing
```bash
cargo test                   # Run all tests
cargo test test_name         # Run specific test by name
cargo test module::tests     # Run tests in specific module
cargo check                  # Quick compilation check
```

### Database
```bash
docker-compose up -d         # Start development PostgreSQL
sqlx migrate run            # Run database migrations
```

## Architecture

### Core Components

1. **Blueprint System** (`src/storage/blueprint.rs`, `src/processor.rs`)
   - Blueprints define automation workflows with ordered node evaluation
   - Each blueprint has an AT-URI identifier and evaluation order
   - Blueprints are cached in memory/Redis for performance

2. **Node Types** (`src/engine/`)
   - `jetstream_entry`: Filters Jetstream events
   - `webhook_entry`: Filters webhook requests  
   - `periodic_entry`: Triggered by cron schedules
   - `condition`: Boolean flow control using DataLogic
   - `transform`: Data transformation using DataLogic
   - `publish_record`: Creates AT Protocol records
   - `publish_webhook`: Sends webhook notifications
   - `facet_text`: Processes text with facets/mentions
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

## Key Configuration

Environment variables control the service behavior:
- `EXTERNAL_BASE`: Service base URL
- `HTTP_COOKIE_KEY`: Cookie encryption key (generate with `openssl rand -base64 64`)
- `ISSUER_DID`: Service DID
- `AIP_HOSTNAME`, `AIP_CLIENT_ID`, `AIP_CLIENT_SECRET`: OAuth configuration
- `JETSTREAM_ENABLED`: Enable Jetstream consumer
- `REDIS_URL`: Redis connection (optional, enables advanced features)

## API Endpoints

XRPC endpoints for blueprint management:
- `GET /xrpc/tools.graze.ifthisthenat.getBlueprints`
- `GET /xrpc/tools.graze.ifthisthenat.getBlueprint`
- `POST /xrpc/tools.graze.ifthisthenat.updateBlueprint`
- `POST /xrpc/tools.graze.ifthisthenat.deleteBlueprint`

See `API.md` for detailed blueprint structure and node payload examples.

## DataLogic Expression System

The engine uses DataLogic for dynamic evaluation. Common patterns:
- Value extraction: `{"val": ["path", "to", "field"]}`
- Comparisons: `{"==": [{"val": ["field"]}, "value"]}`
- String operations: `{"cat": ["prefix", {"val": ["field"]}, "suffix"]}`
- Time functions: `{"now": []}`, `{"datetime": [timestamp]}`

## Performance Considerations

- Blueprint cache reloads are rate-limited
- Use Redis for production deployments with multiple instances
- Jetstream partitioning supports horizontal scaling
- Webhook queue prevents blocking on slow endpoints

## Error Handling

All error strings must use this format:

    error-iftta-<domain>-<number> <message>: <details>

Example errors:

* error-iftta-resolve-1 Multiple DIDs resolved for method
* error-iftta-plc-1 HTTP request failed: https://google.com/ Not Found
* error-iftta-key-1 Error decoding key: invalid

Errors should be represented as enums using the `thiserror` library.

Avoid creating new errors with the `anyhow!(...)` macro.