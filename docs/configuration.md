# Configuration Guide

This guide covers all configuration options for the ifthisthenat application, including environment variables, database configuration, security settings, and monitoring options.

## Table of Contents

- [Quick Start](#quick-start)
- [Required Configuration](#required-configuration)
- [Core Services](#core-services)
- [Queue Configuration](#queue-configuration)
- [Storage Options](#storage-options)
- [Security Settings](#security-settings)
- [Monitoring and Observability](#monitoring-and-observability)
- [Performance Tuning](#performance-tuning)
- [Advanced Features](#advanced-features)
- [Configuration Validation](#configuration-validation)

## Quick Start

1. Copy the production environment template:
   ```bash
   cp docs/production.env .env
   ```

2. Generate secure secrets:
   ```bash
   # Generate cookie key
   echo "HTTP_COOKIE_KEY=$(openssl rand -base64 66)" >> .env

   # Generate database password
   echo "POSTGRES_PASSWORD=$(openssl rand -base64 32)" >> .env
   ```

3. Configure required fields in `.env`:
   - `EXTERNAL_BASE`: Your public domain
   - `ISSUER_DID`: Your AT Protocol DID
   - `AIP_*`: OAuth provider settings

## Required Configuration

These environment variables **must** be set for the application to start:

### Core Application Settings

```bash
# External base URL - must be accessible from the internet
EXTERNAL_BASE=https://your-domain.com

# HTTP server configuration
HTTP_PORT=8080                    # Default: 8080
HTTP_STATIC_PATH=/app/static      # Default: /app/static

# Secure cookie key (generate with: openssl rand -base64 66)
# Must be exactly 88 base64 characters (66 bytes)
HTTP_COOKIE_KEY=your-base64-cookie-key-here

# Database connection
DATABASE_URL=postgres://user:password@postgres:5432/ifthisthenat
```

### AT Protocol Configuration

```bash
# Your application's DID
ISSUER_DID=did:plc:your-issuer-did-here

# PLC directory hostname (usually keep default)
PLC_HOSTNAME=plc.directory        # Default: plc.directory
```

### OAuth Configuration

```bash
# OAuth provider configuration
AIP_HOSTNAME=https://your-aip-provider.com
AIP_CLIENT_ID=your-oauth-client-id
AIP_CLIENT_SECRET=your-oauth-client-secret

# OAuth scopes define permissions when users authenticate
# Important: These are validated against ALLOWED_PUBLISH_COLLECTIONS at startup
AIP_OAUTH_SCOPE=openid email profile atproto account:email
```

#### OAuth Scope Requirements

OAuth scopes determine what permissions your service has when users authenticate:

- `openid` - OpenID Connect basic profile
- `email` - User's email address
- `profile` - User's profile information
- `atproto` - AT Protocol specific permissions
- `account:email` - Read email from user's PDS
- `repo:*` - Full repository access (all collections)
- `repo:{collection}` - Access to specific collection
- `transition:generic` - Transition scope with broad permissions

**Important**: If the `publish_record` node type is enabled, OAuth scopes must include repository permissions. Either use `repo:*`/`transition:generic` for full access, or include specific `repo:{collection}` scopes for each collection in `ALLOWED_PUBLISH_COLLECTIONS`.

## Core Services

### PostgreSQL Database

```bash
# Database connection parameters
POSTGRES_DB=ifthisthenat
POSTGRES_USER=ifthisthenat_user
POSTGRES_PASSWORD=secure-database-password
POSTGRES_PORT=5432               # Default: 5432
```

### Redis (Optional but Recommended)

Redis enables distributed features and improved performance:

```bash
# Redis connection
REDIS_URL=redis://redis:6379

# Jetstream cursor storage
REDIS_CURSOR_KEY=ifthisthenat:jetstream:cursor    # Default
REDIS_CURSOR_TTL_SECONDS=86400                    # Default: 24 hours
```

### Jetstream (AT Protocol Firehose)

```bash
# Enable Jetstream consumer
JETSTREAM_ENABLED=true           # Default: false

# Jetstream server
JETSTREAM_HOSTNAME=jetstream2.us-east.bsky.network

# Collections to monitor (comma-separated, empty = all)
JETSTREAM_COLLECTIONS=app.bsky.feed.post,app.bsky.feed.like

# Distributed processing
JETSTREAM_INSTANCE_ID=0          # Default: 0 (must be < total instances)
JETSTREAM_TOTAL_INSTANCES=1      # Default: 1
JETSTREAM_WORKER_THREADS=2       # Default: 1

# Partition strategy: did, collection, round-robin, or custom:{field}
JETSTREAM_PARTITION_STRATEGY=did # Default: did

# File-based cursor (alternative to Redis)
JETSTREAM_CURSOR_PATH=/app/data/cursor.json
```

## Queue Configuration

### Blueprint Queue

Choose between MPSC (single instance) or Redis (distributed):

```bash
# Queue adapter type
BLUEPRINT_QUEUE_ADAPTER=mpsc     # Options: mpsc, redis

# MPSC settings
BLUEPRINT_QUEUE_BUFFER_SIZE=5000 # Default: 5000

# Redis queue settings (when adapter=redis)
BLUEPRINT_QUEUE_REDIS_PREFIX=queue:blueprint:
BLUEPRINT_QUEUE_REDIS_WORKER_ID=worker-001

# Queue monitoring
BLUEPRINT_QUEUE_HEALTH_CHECK=true    # Default: true
BLUEPRINT_QUEUE_HEALTH_INTERVAL=60   # Default: 60
BLUEPRINT_QUEUE_MAX_RETRIES=3        # Default: 3
```

### Webhook Queue

```bash
# Enable async webhook processing
WEBHOOK_QUEUE_ENABLED=true           # Default: false

# Concurrency and performance
WEBHOOK_MAX_CONCURRENT=10            # Default: 10
WEBHOOK_DEFAULT_TIMEOUT_MS=30000     # Default: 30000 (30 seconds)
WEBHOOK_QUEUE_SIZE=1000              # Default: 1000

# Retry configuration
WEBHOOK_MAX_RETRIES=3                # Default: 3
WEBHOOK_RETRY_DELAY_MS=1000          # Default: 1000 (1 second)

# Logging (be careful with sensitive data)
WEBHOOK_LOG_BODIES=false             # Default: false
```

## Storage Options

### File-based Evaluation Storage

Store evaluation results as JSON files for audit trails:

```bash
# Enable filesystem logging of evaluation results
EVALUATION_LOG_DIRECTORY=/var/log/iftta/logs
```

When enabled, evaluation results are stored in:
```
/var/log/iftta/logs/{did}/{blueprint_id}/{evaluation_id}.json
```

### Denylist Storage

The denylist system uses PostgreSQL with bloom filter optimization:

- **PostgreSQL**: Full-featured with bloom filter
- **No-op**: Disabled for testing

Configuration is automatic based on database availability.

## Security Settings

### Access Control

```bash
# Admin access (semicolon-separated DID list)
ADMIN_DIDS=did:plc:admin1;did:plc:admin2

# Disabled node types (comma-separated, empty = allow all)
# Valid types: jetstream_entry, webhook_entry, periodic_entry, condition,
#              transform, facet_text, publish_record, publish_webhook, debug_action
DISABLED_NODE_TYPES=debug_action

# Collection constraints for publish_record nodes
# When specified, publish_record nodes can only publish to these collections
# OAuth scopes must include 'repo:{collection}' for each allowed collection
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post,app.bsky.feed.like
```

### Network Security

```bash
# HTTP client timeout
HTTP_CLIENT_TIMEOUT=30              # Default: 8 seconds
```

## Monitoring and Observability

### Sentry Integration

Optional error tracking and performance monitoring:

```bash
# Sentry configuration (optional)
SENTRY_DSN=https://your-sentry-dsn@sentry.io/project-id
SENTRY_ENVIRONMENT=production       # Default: development
SENTRY_TRACES_SAMPLE_RATE=0.1       # Default: 0.1 (10% of traces)
SENTRY_DEBUG=false                  # Default: false
```

### Logging

```bash
# Rust logging configuration
RUST_LOG=ifthisthenat=info,warn,error
RUST_BACKTRACE=1                    # Enable backtraces

# User agent for HTTP requests
USER_AGENT=ifthisthenat/1.0.0       # Default: ifthisthenat/{version}
```

## Performance Tuning

### Caching

```bash
# Identity resolution cache
IDENTITY_CACHE_SIZE=10000           # Default: 1000
IDENTITY_CACHE_TTL_MINUTES=30       # Default: 30

# Blueprint cache
BLUEPRINT_CACHE_RELOAD_SECONDS=30   # Default: 30
```

### High Traffic Configuration

For high-volume deployments:

```bash
# Increase worker threads
JETSTREAM_WORKER_THREADS=4

# Increase webhook concurrency
WEBHOOK_MAX_CONCURRENT=20
WEBHOOK_QUEUE_SIZE=5000

# Increase cache sizes
IDENTITY_CACHE_SIZE=50000
BLUEPRINT_QUEUE_BUFFER_SIZE=10000
```

### Resource-Constrained Configuration

For limited resources:

```bash
# Reduce worker threads
JETSTREAM_WORKER_THREADS=1

# Reduce concurrency
WEBHOOK_MAX_CONCURRENT=5
WEBHOOK_QUEUE_SIZE=500

# Reduce cache sizes
IDENTITY_CACHE_SIZE=1000
BLUEPRINT_QUEUE_BUFFER_SIZE=1000
```

## Advanced Features

### Scheduler (Periodic Entry Nodes)

```bash
# Enable scheduler for periodic entry nodes
SCHEDULER_ENABLED=false             # Default: false

# Scheduler timing
SCHEDULER_CHECK_INTERVAL_SECS=10    # Default: 10
SCHEDULER_CACHE_RELOAD_SECS=60      # Default: 60
SCHEDULER_MAX_CONCURRENT=5          # Default: 5

# Debug logging
SCHEDULER_DEBUG_LOGGING=false       # Default: false
```

### Leadership Election (Distributed Deployments)

For coordinating tasks across multiple instances:

```bash
# Global leadership settings (apply to all tasks unless overridden)
LEADERSHIP_ELECTION_KEY=ifthisthenat:leadership:default
LEADERSHIP_RETRY_INTERVAL_SECS=30   # Minimum: 5 seconds
LEADERSHIP_TTL_SECS=120             # Minimum: 30 seconds
LEADERSHIP_INSTANCE_ID=instance-001 # Auto-generated if not set

# Task-specific leadership (independent elections)
BLUEPRINT_LEADERSHIP_ENABLED=false
WEBHOOK_LEADERSHIP_ENABLED=false
SCHEDULER_LEADERSHIP_ENABLED=false

# Override settings per task type
BLUEPRINT_LEADERSHIP_ELECTION_KEY=ifthisthenat:leadership:blueprint
WEBHOOK_LEADERSHIP_ELECTION_KEY=ifthisthenat:leadership:webhook
SCHEDULER_LEADERSHIP_ELECTION_KEY=ifthisthenat:leadership:scheduler
```

### Evaluation Tracing

```bash
# Enable traced evaluation (enhanced observability)
TRACED_EVALUATION=true
```

## Configuration Validation

The application performs comprehensive validation at startup:

### OAuth Scope Validation

If `publish_record` node type is enabled, the system validates:

1. **Repository Access Required**: OAuth scopes must include at least one `repo:` permission
2. **Blanket Permissions**: `repo:*` or `transition:generic` satisfy all collection requirements
3. **Specific Collections**: Each collection in `ALLOWED_PUBLISH_COLLECTIONS` must have a corresponding `repo:{collection}` scope

**Examples:**

✅ **Valid Configurations:**
```bash
# Blanket repo access
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post,app.bsky.feed.like
AIP_OAUTH_SCOPE=openid profile repo:*

# Specific collection scopes match
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post
AIP_OAUTH_SCOPE=openid profile repo:app.bsky.feed.post
```

❌ **Invalid Configurations:**
```bash
# Missing repo scope for app.bsky.feed.like
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post,app.bsky.feed.like
AIP_OAUTH_SCOPE=openid profile repo:app.bsky.feed.post

# No repo permissions at all
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post
AIP_OAUTH_SCOPE=openid profile account:email
```

### Configuration Checklist

Before deploying, ensure you have:

- ✅ Replaced `EXTERNAL_BASE` with your actual domain
- ✅ Generated and set `HTTP_COOKIE_KEY` with: `openssl rand -base64 66`
- ✅ Set `DATABASE_URL` with secure credentials
- ✅ Configured `ISSUER_DID` with your AT Protocol DID
- ✅ Set OAuth credentials (`AIP_HOSTNAME`, `AIP_CLIENT_ID`, `AIP_CLIENT_SECRET`)
- ✅ Configured `AIP_OAUTH_SCOPE` with appropriate repository permissions
- ✅ Validated OAuth scopes are compatible with `ALLOWED_PUBLISH_COLLECTIONS`
- ✅ Generated secure `POSTGRES_PASSWORD`
- ✅ Configured `ADMIN_DIDS` with authorized administrator DIDs
- ✅ Reviewed `DISABLED_NODE_TYPES` for security requirements
- ✅ Set `ALLOWED_PUBLISH_COLLECTIONS` if collection constraints are needed
- ✅ Reviewed and adjusted performance settings for your environment
- ✅ Set up monitoring with `SENTRY_DSN` if using Sentry
- ✅ Verified Redis configuration if using distributed features
- ✅ Configured scheduler settings if using periodic entry nodes
- ✅ Configured leadership elections for distributed deployments

### Security Reminders

- ✅ Set file permissions to 600: `chmod 600 .env`
- ✅ Never commit this file with real credentials to version control
- ✅ Regularly rotate secrets and passwords
- ✅ Use HTTPS/TLS in production
- ✅ Monitor logs for security events
- ✅ Use unique `LEADERSHIP_INSTANCE_ID` values for each instance

## Troubleshooting

### Common Configuration Errors

**OAuth Scope Validation Errors:**
```bash
# Error: Collection allowed but OAuth scope missing
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post,app.bsky.feed.like
AIP_OAUTH_SCOPE=openid profile repo:app.bsky.feed.post

# Solutions:
# 1. Add missing scope
AIP_OAUTH_SCOPE="openid profile repo:app.bsky.feed.post repo:app.bsky.feed.like"

# 2. Use blanket permission
AIP_OAUTH_SCOPE="openid profile repo:*"

# 3. Remove collection constraint
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post
```

**Application Won't Start:**
1. Check environment variables are set correctly
2. Verify database connectivity
3. Check OAuth credentials and DID validity
4. Review logs for specific error messages

**Memory/Performance Issues:**
- Reduce cache sizes for memory-constrained environments
- Increase queue sizes for high-throughput scenarios
- Adjust worker thread counts based on available CPU cores

For more detailed troubleshooting, see the production deployment guide.