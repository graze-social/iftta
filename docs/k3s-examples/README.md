# k3s Local Development Examples

This directory contains all the necessary files and scripts for deploying ifthisthenat on a local k3s cluster for development and integration testing.

## Quick Start

```bash
# Make scripts executable
chmod +x setup-k3s.sh integration-test.sh

# Run automated setup (interactive menu)
./setup-k3s.sh

# Or run full setup directly
./setup-k3s.sh --full
```

## Files Overview

### Configuration Files

#### `values-local-dev.yaml`
Helm values file optimized for local k3s development with:
- PostgreSQL and Redis enabled
- Resource limits suitable for local development
- Debug logging enabled
- Local storage configuration

#### `local-storage.yaml`
Kubernetes manifest for creating persistent volumes using local storage:
- 10Gi for PostgreSQL data
- 2Gi for Redis data
- StorageClass configuration for k3s

#### `init-db.sql`
PostgreSQL initialization script that creates:
- All required database tables
- Indexes for performance
- Triggers for automatic timestamp updates
- Proper permissions

### Scripts

#### `setup-k3s.sh`
Automated setup script with interactive menu:
- Creates k3d cluster with proper configuration
- Installs Traefik ingress controller
- Deploys PostgreSQL and Redis
- Builds and imports Docker image
- Deploys ifthisthenat application
- Initializes database schema
- Configures local DNS

**Usage:**
```bash
./setup-k3s.sh          # Interactive menu
./setup-k3s.sh --help   # Show help
./setup-k3s.sh --cleanup # Remove everything
```

#### `integration-test.sh`
Comprehensive integration testing script that validates:
- Kubernetes resources deployment
- PostgreSQL connectivity and operations
- Redis connectivity and operations
- Application health and endpoints
- API functionality
- Resource limits and security settings
- Performance benchmarks

**Usage:**
```bash
./integration-test.sh

# With custom namespace
NAMESPACE=my-namespace ./integration-test.sh

# With custom base URL
BASE_URL=http://my-app:8080 ./integration-test.sh
```

### Testing Files

#### `k6-load-test.js`
k6 load testing script for performance testing:
- Gradual ramp-up to 20 concurrent users
- Tests all major endpoints
- Measures response times and error rates
- Generates HTML and JSON reports

**Usage:**
```bash
# Install k6 first
brew install k6  # macOS

# Run load test
k6 run k6-load-test.js

# With custom base URL
k6 run -e BASE_URL=http://localhost:8080 k6-load-test.js

# With authentication token
k6 run -e AUTH_TOKEN=your-token k6-load-test.js
```

### Alternative Setup

#### `docker-compose.yaml`
Docker Compose configuration for simpler local testing without k3s:
- Complete stack with PostgreSQL, Redis, and application
- Optional debug tools (Adminer, RedisInsight)
- Suitable for quick local development

**Usage:**
```bash
# Start all services
docker-compose up -d

# Start with debug tools
docker-compose --profile debug up -d

# View logs
docker-compose logs -f ifthisthenat

# Stop everything
docker-compose down

# Stop and remove volumes
docker-compose down -v
```

## Common Workflows

### 1. First Time Setup
```bash
# Run full automated setup
./setup-k3s.sh
# Select option 1 (Full Setup)

# Verify deployment
kubectl get pods -n ifthisthenat-dev

# Run integration tests
./integration-test.sh
```

### 2. Deploy Changes
```bash
# Rebuild Docker image
docker build -t ifthisthenat:latest .

# Import to k3s
k3d image import ifthisthenat:latest -c ifthisthenat-dev

# Upgrade Helm release
helm upgrade ifthisthenat ./charts/ifthisthenat \
  -n ifthisthenat-dev \
  -f values-local-dev.yaml

# Verify
kubectl rollout status deployment/ifthisthenat -n ifthisthenat-dev
```

### 3. Database Operations
```bash
# Get PostgreSQL pod
POSTGRES_POD=$(kubectl get pods -n ifthisthenat-dev \
  -l app.kubernetes.io/name=postgresql \
  -o jsonpath="{.items[0].metadata.name}")

# Connect to database
kubectl exec -it $POSTGRES_POD -n ifthisthenat-dev -- \
  psql -U ifthisthenat -d ifthisthenat_dev

# Run migrations
kubectl cp migrations.sql ifthisthenat-dev/$POSTGRES_POD:/tmp/
kubectl exec $POSTGRES_POD -n ifthisthenat-dev -- \
  psql -U ifthisthenat -d ifthisthenat_dev -f /tmp/migrations.sql
```

### 4. Debugging
```bash
# View application logs
kubectl logs -f deployment/ifthisthenat -n ifthisthenat-dev

# Execute into application pod
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-dev -- /bin/sh

# Port forward for direct access
kubectl port-forward svc/ifthisthenat 8080:8080 -n ifthisthenat-dev

# Check events
kubectl get events -n ifthisthenat-dev --sort-by='.lastTimestamp'
```

### 5. Performance Testing
```bash
# Run quick load test
k6 run --duration 1m --vus 5 k6-load-test.js

# Run full load test
k6 run k6-load-test.js

# Generate HTML report
k6 run --out json=results.json k6-load-test.js
k6-to-html results.json report.html
```

## Troubleshooting

### Cluster Issues
```bash
# Check cluster status
k3d cluster list
kubectl cluster-info

# Restart cluster
k3d cluster stop ifthisthenat-dev
k3d cluster start ifthisthenat-dev
```

### Pod Not Starting
```bash
# Check pod events
kubectl describe pod <pod-name> -n ifthisthenat-dev

# Check resource availability
kubectl top nodes
kubectl describe nodes
```

### Database Connection Issues
```bash
# Test from application pod
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-dev -- \
  pg_isready -h ifthisthenat-postgresql -U ifthisthenat

# Check PostgreSQL logs
kubectl logs deployment/ifthisthenat-postgresql -n ifthisthenat-dev
```

### Image Issues
```bash
# List images in k3d
docker exec k3d-ifthisthenat-dev-agent-0 crictl images

# Force reimport
docker save ifthisthenat:latest | \
  docker exec -i k3d-ifthisthenat-dev-agent-0 ctr images import -
```

## Requirements

### Tools
- Docker Desktop or Docker Engine
- kubectl (1.19+)
- Helm (3.0+)
- k3d (5.0+)
- curl
- openssl

### System Resources
- Minimum: 4GB RAM, 10GB disk space
- Recommended: 8GB RAM, 20GB disk space

### Optional Tools
- k6 (for load testing)
- jq (for JSON parsing in scripts)
- psql (for direct database access)
- redis-cli (for direct Redis access)

## Clean Up

```bash
# Remove everything
./setup-k3s.sh --cleanup

# Or manually:
helm uninstall ifthisthenat -n ifthisthenat-dev
kubectl delete namespace ifthisthenat-dev
k3d cluster delete ifthisthenat-dev
rm -rf /tmp/k3s-data
```

## Additional Resources

- [k3s Documentation](https://k3s.io/)
- [k3d Documentation](https://k3d.io/)
- [Helm Documentation](https://helm.sh/docs/)
- [k6 Documentation](https://k6.io/docs/)
- [Main Deployment Guide](../k3s-local-deployment.md)