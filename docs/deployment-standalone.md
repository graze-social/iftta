# Standalone Production Deployment Guide

This guide covers deploying ifthisthenat in a production environment using Docker and Docker Compose for standalone deployments.

## Table of Contents

- [Quick Start](#quick-start)
- [Architecture Overview](#architecture-overview)
- [Prerequisites](#prerequisites)
- [Deployment Steps](#deployment-steps)
- [Configuration](#configuration)
- [Security Considerations](#security-considerations)
- [Monitoring and Logging](#monitoring-and-logging)
- [Backup and Recovery](#backup-and-recovery)
- [Scaling Considerations](#scaling-considerations)
- [Troubleshooting](#troubleshooting)
- [Maintenance](#maintenance)

## Quick Start

1. **Copy the production environment file:**
   ```bash
   cp docs/production.env .env
   ```

2. **Configure required environment variables:**
   ```bash
   # Generate secure cookie key
   echo "HTTP_COOKIE_KEY=$(openssl rand -base64 66)" >> .env

   # Generate secure database password
   echo "POSTGRES_PASSWORD=$(openssl rand -base64 32)" >> .env

   # Edit .env with your production values
   nano .env
   ```

3. **Deploy using Docker Compose:**
   ```bash
   docker-compose -f docker-compose.prod.yml up -d
   ```

## Architecture Overview

ifthisthenat is an AT Protocol automation tool with the following components:

- **Web Server**: HTTP API and static file serving
- **Jetstream Consumer**: Processes AT Protocol firehose events
- **Blueprint Engine**: Executes automation blueprints
- **Queue System**: Configurable queue adapters (MPSC/Redis)
- **Webhook Processor**: Handles outbound webhook calls
- **Database**: PostgreSQL for persistent data
- **Cache/Queue**: Redis for distributed operations (optional)

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Load Balancer │───▶│   ifthisthenat   │───▶│   PostgreSQL    │
│   (nginx/traefik)│    │   Application    │    │   Database      │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                │
                                ▼
                       ┌──────────────────┐
                       │      Redis       │
                       │   (Optional)     │
                       └──────────────────┘
```

## Prerequisites

### System Requirements
- **CPU**: 2+ cores recommended
- **Memory**: 4GB minimum, 8GB recommended
- **Storage**: 20GB minimum, SSD recommended
- **Network**: Stable internet connection for AT Protocol events

### Required Software
- Docker (20.10+) and Docker Compose (1.29+)
- OpenSSL (for generating secrets)
- PostgreSQL client (for database operations)

### Required Credentials
- Valid AT Protocol DID and OAuth credentials
- Domain name with SSL certificate
- SMTP credentials (optional, for notifications)

## Deployment Steps

### 1. Prepare Environment

```bash
# Create project directory
mkdir -p /opt/ifthisthenat
cd /opt/ifthisthenat

# Clone repository
git clone https://github.com/your-org/ifthisthenat.git .

# Copy and configure environment
cp docs/production.env .env
```

### 2. Generate Secrets

```bash
# Generate secure cookie key (exactly 88 characters)
export COOKIE_KEY=$(openssl rand -base64 66)
echo "HTTP_COOKIE_KEY=$COOKIE_KEY" >> .env

# Generate secure database password
export DB_PASSWORD=$(openssl rand -base64 32)
echo "POSTGRES_PASSWORD=$DB_PASSWORD" >> .env

# Update DATABASE_URL with generated password
sed -i "s/REPLACE_WITH_SECURE_PASSWORD/$DB_PASSWORD/g" .env
```

### 3. Configure Production Settings

Edit `.env` and configure these required fields:

```bash
# Core settings
EXTERNAL_BASE=https://your-domain.com
ISSUER_DID=did:plc:your-issuer-did-here

# OAuth configuration
AIP_HOSTNAME=https://your-aip-provider.com
AIP_CLIENT_ID=your-oauth-client-id
AIP_CLIENT_SECRET=your-oauth-client-secret

# Admin access
ADMIN_DIDS=did:plc:your-admin-did-1;did:plc:your-admin-did-2
```

### 4. Deploy with Docker Compose

Create production Docker Compose file (`docker-compose.prod.yml`):

```yaml
version: '3.8'

services:
  postgres:
    image: postgres:15-alpine
    restart: unless-stopped
    environment:
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - postgres_data:/var/lib/postgresql/data
    networks:
      - app-network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${POSTGRES_USER} -d ${POSTGRES_DB}"]
      interval: 30s
      timeout: 10s
      retries: 3

  redis:
    image: redis:7-alpine
    restart: unless-stopped
    command: redis-server --appendonly yes
    volumes:
      - redis_data:/data
    networks:
      - app-network
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 30s
      timeout: 10s
      retries: 3

  ifthisthenat:
    build: .
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
    env_file:
      - .env
    ports:
      - "8080:8080"
    networks:
      - app-network
    volumes:
      - app_logs:/var/log/ifthisthenat
      - app_data:/app/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 60s

networks:
  app-network:
    driver: bridge

volumes:
  postgres_data:
  redis_data:
  app_logs:
  app_data:
```

Start the services:

```bash
# Build and start services
docker-compose -f docker-compose.prod.yml up -d

# Check logs
docker-compose -f docker-compose.prod.yml logs -f

# Check service health
docker-compose -f docker-compose.prod.yml ps
```

### 5. Database Migration

```bash
# Wait for PostgreSQL to be ready
docker-compose -f docker-compose.prod.yml exec postgres \
  pg_isready -U ifthisthenat -d ifthisthenat

# Run migrations (if you have migration files)
docker-compose -f docker-compose.prod.yml exec postgres \
  psql -U ifthisthenat -d ifthisthenat -f /migrations/init.sql
```

### 6. Configure Reverse Proxy

Example nginx configuration:

```nginx
# /etc/nginx/sites-available/ifthisthenat
server {
    listen 80;
    server_name your-domain.com;
    return 301 https://$server_name$request_uri;
}

server {
    listen 443 ssl http2;
    server_name your-domain.com;

    ssl_certificate /etc/ssl/certs/your-domain.com.crt;
    ssl_certificate_key /etc/ssl/private/your-domain.com.key;

    # SSL configuration
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers ECDHE-RSA-AES256-GCM-SHA512:DHE-RSA-AES256-GCM-SHA512;
    ssl_prefer_server_ciphers off;
    ssl_session_cache shared:SSL:10m;

    # Security headers
    add_header X-Frame-Options DENY;
    add_header X-Content-Type-Options nosniff;
    add_header X-XSS-Protection "1; mode=block";
    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket support (if needed)
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";

        # Timeouts
        proxy_connect_timeout 60s;
        proxy_send_timeout 60s;
        proxy_read_timeout 60s;
    }

    # Health check endpoint
    location /health {
        proxy_pass http://127.0.0.1:8080/health;
        access_log off;
    }
}
```

Enable the site:
```bash
ln -s /etc/nginx/sites-available/ifthisthenat /etc/nginx/sites-enabled/
nginx -t
systemctl reload nginx
```

## Configuration

### Production Environment Settings

#### High Traffic Configuration
```bash
# Increase worker threads
JETSTREAM_WORKER_THREADS=4

# Increase webhook concurrency
WEBHOOK_MAX_CONCURRENT=20
WEBHOOK_QUEUE_SIZE=5000

# Increase cache sizes
IDENTITY_CACHE_SIZE=50000
BLUEPRINT_QUEUE_BUFFER_SIZE=10000

# Enable Redis for distributed processing
REDIS_URL=redis://redis:6379
BLUEPRINT_QUEUE_ADAPTER=redis
```

#### Resource-Constrained Configuration
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

#### Scheduler Configuration

For periodic entry nodes:

```bash
# Enable scheduler
SCHEDULER_ENABLED=true

# Configure timing
SCHEDULER_CHECK_INTERVAL_SECS=10
SCHEDULER_CACHE_RELOAD_SECS=60
SCHEDULER_MAX_CONCURRENT=5
```

## Security Considerations

### Environment Security

```bash
# Set secure file permissions
chmod 600 .env
chown root:root .env

# Use Docker secrets for sensitive values
echo "your-oauth-secret" | docker secret create oauth_secret -
```

### Network Security

```bash
# Configure firewall
ufw allow ssh
ufw allow 80/tcp
ufw allow 443/tcp
ufw deny 8080/tcp  # Block direct access to app
ufw enable

# Use private networks for database
# Configure PostgreSQL to only accept connections from app containers
```

### Application Security

```bash
# Configure security settings in .env
DISABLED_NODE_TYPES=debug_action  # Disable debug in production
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post,app.bsky.feed.like

# Monitor and rotate secrets
openssl rand -base64 66 > new_cookie_key.txt
# Update .env and restart services
```

### SSL/TLS Configuration

```bash
# Use Let's Encrypt for free SSL certificates
certbot --nginx -d your-domain.com

# Or use custom certificates
cp your-domain.com.crt /etc/ssl/certs/
cp your-domain.com.key /etc/ssl/private/
chmod 644 /etc/ssl/certs/your-domain.com.crt
chmod 600 /etc/ssl/private/your-domain.com.key
```

## Monitoring and Logging

### Application Logs

```bash
# View application logs
docker-compose -f docker-compose.prod.yml logs -f ifthisthenat

# View specific service logs
docker-compose -f docker-compose.prod.yml logs postgres
docker-compose -f docker-compose.prod.yml logs redis
```

### Log Management

Configure log rotation to prevent disk space issues:

```bash
# Create logrotate configuration
cat > /etc/logrotate.d/ifthisthenat << EOF
/var/log/ifthisthenat/*.log {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
}
EOF
```

### Health Monitoring

Create health check script:

```bash
#!/bin/bash
# /usr/local/bin/ifthisthenat-health.sh

# Check application health
if curl -f http://localhost:8080/health > /dev/null 2>&1; then
    echo "OK: Application is healthy"
else
    echo "ERROR: Application health check failed"
    exit 1
fi

# Check database connectivity
if docker-compose -f /opt/ifthisthenat/docker-compose.prod.yml \
   exec -T postgres pg_isready -U ifthisthenat > /dev/null 2>&1; then
    echo "OK: Database is accessible"
else
    echo "ERROR: Database connectivity failed"
    exit 1
fi

# Check Redis connectivity
if docker-compose -f /opt/ifthisthenat/docker-compose.prod.yml \
   exec -T redis redis-cli ping > /dev/null 2>&1; then
    echo "OK: Redis is accessible"
else
    echo "ERROR: Redis connectivity failed"
    exit 1
fi
```

### Sentry Integration

Enable error tracking:

```bash
# Add to .env
SENTRY_DSN=https://your-sentry-dsn@sentry.io/project-id
SENTRY_ENVIRONMENT=production
SENTRY_TRACES_SAMPLE_RATE=0.01  # Sample 1% of traces
```

## Backup and Recovery

### Database Backups

```bash
#!/bin/bash
# /usr/local/bin/backup-database.sh

BACKUP_DIR="/opt/backups/ifthisthenat"
DATE=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="$BACKUP_DIR/database_$DATE.sql"

# Create backup directory
mkdir -p $BACKUP_DIR

# Create database backup
docker-compose -f /opt/ifthisthenat/docker-compose.prod.yml exec -T postgres \
    pg_dump -U ifthisthenat ifthisthenat > $BACKUP_FILE

# Compress backup
gzip $BACKUP_FILE

# Remove backups older than 30 days
find $BACKUP_DIR -name "database_*.sql.gz" -mtime +30 -delete

echo "Backup completed: ${BACKUP_FILE}.gz"
```

Set up automated backups:

```bash
# Add to crontab
crontab -e

# Add this line for daily backups at 2 AM
0 2 * * * /usr/local/bin/backup-database.sh
```

### Configuration Backups

```bash
#!/bin/bash
# /usr/local/bin/backup-config.sh

BACKUP_DIR="/opt/backups/ifthisthenat-config"
DATE=$(date +%Y%m%d_%H%M%S)

mkdir -p $BACKUP_DIR

# Backup configuration files (excluding sensitive data)
tar -czf "$BACKUP_DIR/config_$DATE.tar.gz" \
    -C /opt/ifthisthenat \
    docker-compose.prod.yml \
    Dockerfile \
    --exclude='.env'

# Keep only last 10 config backups
ls -t $BACKUP_DIR/config_*.tar.gz | tail -n +11 | xargs rm -f
```

### Recovery Procedures

```bash
# Restore database from backup
gunzip database_20241201_020000.sql.gz
docker-compose -f docker-compose.prod.yml exec -T postgres \
    psql -U ifthisthenat ifthisthenat < database_20241201_020000.sql

# Restore application
docker-compose -f docker-compose.prod.yml down
tar -xzf config_20241201_020000.tar.gz
docker-compose -f docker-compose.prod.yml up -d
```

## Scaling Considerations

### Horizontal Scaling

For multiple instances:

```bash
# Use Redis for distributed queuing
BLUEPRINT_QUEUE_ADAPTER=redis
REDIS_URL=redis://redis-cluster:6379

# Configure distributed Jetstream processing
JETSTREAM_INSTANCE_ID=0
JETSTREAM_TOTAL_INSTANCES=3
JETSTREAM_PARTITION_STRATEGY=did

# Enable leadership election
BLUEPRINT_LEADERSHIP_ENABLED=true
WEBHOOK_LEADERSHIP_ENABLED=true
SCHEDULER_LEADERSHIP_ENABLED=true
```

### Load Balancer Configuration

Example HAProxy configuration:

```
backend ifthisthenat_backend
    balance roundrobin
    option httpchk GET /health
    server app1 10.0.1.10:8080 check
    server app2 10.0.1.11:8080 check
    server app3 10.0.1.12:8080 check

frontend ifthisthenat_frontend
    bind *:80
    redirect scheme https if !{ ssl_fc }

frontend ifthisthenat_ssl_frontend
    bind *:443 ssl crt /etc/ssl/certs/your-domain.com.pem
    default_backend ifthisthenat_backend
```

### Database Scaling

For high load:

```bash
# Use connection pooling
DATABASE_URL=postgres://user:pass@pgbouncer:6432/ifthisthenat

# Configure read replicas
DATABASE_READ_URL=postgres://user:pass@postgres-replica:5432/ifthisthenat
```

## Troubleshooting

### Common Issues

#### Application Won't Start

1. **Check environment variables:**
   ```bash
   docker-compose -f docker-compose.prod.yml config
   ```

2. **Verify database connectivity:**
   ```bash
   docker-compose -f docker-compose.prod.yml exec postgres \
     pg_isready -U ifthisthenat -d ifthisthenat
   ```

3. **Check OAuth credentials:**
   ```bash
   curl -v "https://${AIP_HOSTNAME}/.well-known/openid_configuration"
   ```

#### OAuth Scope Validation Errors

```bash
# Example error:
# Collection 'app.bsky.feed.like' is allowed but OAuth scopes missing 'repo:app.bsky.feed.like'

# Solutions:
# 1. Add missing scope
AIP_OAUTH_SCOPE="openid profile repo:app.bsky.feed.post repo:app.bsky.feed.like"

# 2. Use blanket permission
AIP_OAUTH_SCOPE="openid profile repo:*"

# 3. Remove collection constraint
ALLOWED_PUBLISH_COLLECTIONS=app.bsky.feed.post
```

#### High Memory Usage

```bash
# Check memory usage
docker stats

# Reduce cache sizes
IDENTITY_CACHE_SIZE=1000
BLUEPRINT_QUEUE_BUFFER_SIZE=1000

# Monitor queue depths
docker-compose -f docker-compose.prod.yml logs ifthisthenat | grep "queue depth"
```

#### Slow Performance

```bash
# Increase worker threads
JETSTREAM_WORKER_THREADS=4

# Enable Redis for better queue performance
BLUEPRINT_QUEUE_ADAPTER=redis

# Optimize database queries
docker-compose -f docker-compose.prod.yml exec postgres \
  psql -U ifthisthenat -d ifthisthenat -c "
  SELECT query, mean_time, calls
  FROM pg_stat_statements
  ORDER BY mean_time DESC
  LIMIT 10;"
```

### Debug Mode

For troubleshooting, temporarily enable debug logging:

```bash
# Add to .env
RUST_LOG=debug
RUST_BACKTRACE=full

# Restart application
docker-compose -f docker-compose.prod.yml restart ifthisthenat

# View debug logs
docker-compose -f docker-compose.prod.yml logs -f ifthisthenat
```

**Remember to disable debug mode in production after troubleshooting.**

## Maintenance

### Regular Updates

```bash
#!/bin/bash
# /usr/local/bin/update-ifthisthenat.sh

cd /opt/ifthisthenat

# Pull latest changes
git pull origin main

# Backup current deployment
docker-compose -f docker-compose.prod.yml down
cp docker-compose.prod.yml docker-compose.prod.yml.backup

# Build new image
docker-compose -f docker-compose.prod.yml build --no-cache

# Start services
docker-compose -f docker-compose.prod.yml up -d

# Verify health
sleep 30
if curl -f http://localhost:8080/health; then
    echo "Update successful"
else
    echo "Update failed, rolling back"
    docker-compose -f docker-compose.prod.yml down
    cp docker-compose.prod.yml.backup docker-compose.prod.yml
    docker-compose -f docker-compose.prod.yml up -d
fi
```

### Database Maintenance

```bash
# Regular maintenance script
#!/bin/bash
# /usr/local/bin/db-maintenance.sh

# Vacuum and analyze database
docker-compose -f /opt/ifthisthenat/docker-compose.prod.yml exec postgres \
    psql -U ifthisthenat -d ifthisthenat -c "VACUUM ANALYZE;"

# Update statistics
docker-compose -f /opt/ifthisthenat/docker-compose.prod.yml exec postgres \
    psql -U ifthisthenat -d ifthisthenat -c "UPDATE pg_stat_bgwriter;"

echo "Database maintenance completed"
```

### Log Cleanup

```bash
# Clean old Docker logs
docker system prune -f

# Clean old application logs
find /var/log/ifthisthenat -name "*.log" -mtime +30 -delete

# Clean old backups
find /opt/backups -name "*.sql.gz" -mtime +90 -delete
```

### Security Updates

```bash
# Update base images
docker-compose -f docker-compose.prod.yml pull

# Rebuild with latest base images
docker-compose -f docker-compose.prod.yml build --no-cache --pull

# Update system packages
apt update && apt upgrade -y

# Restart services
docker-compose -f docker-compose.prod.yml restart
```

This guide provides a comprehensive foundation for deploying ifthisthenat in production. For Kubernetes deployments, see the separate Kubernetes deployment guide.