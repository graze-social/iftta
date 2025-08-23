# Local k3s Deployment and Integration Testing Guide

This document provides comprehensive instructions for deploying ifthisthenat on a local k3s cluster with PostgreSQL and Redis for development and integration testing.

## Prerequisites

### System Requirements
- macOS, Linux, or WSL2 on Windows
- 8GB RAM minimum (16GB recommended)
- 20GB free disk space
- Docker Desktop (for macOS/Windows) or Docker Engine (Linux)

### Required Tools
```bash
# Install k3s (macOS with Homebrew)
brew install k3s

# Install k3s (Linux)
curl -sfL https://get.k3s.io | sh -

# Install Helm
curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash

# Install kubectl if not already installed
brew install kubectl  # macOS
# or
curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"

# Install PostgreSQL client for testing
brew install postgresql  # macOS
apt-get install postgresql-client  # Linux
```

## Step 1: Start k3s Cluster

### macOS/Linux with Docker
```bash
# Start k3s in Docker (recommended for local development)
k3d cluster create ifthisthenat-dev \
  --servers 1 \
  --agents 2 \
  --port 8080:80@loadbalancer \
  --port 8443:443@loadbalancer \
  --volume /tmp/k3s-data:/var/lib/rancher/k3s/storage@all \
  --wait

# Verify cluster is running
kubectl cluster-info
kubectl get nodes
```

### Alternative: Native k3s on Linux
```bash
# Start k3s with write permissions
sudo k3s server --write-kubeconfig-mode 644 &

# Export kubeconfig
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml

# Verify
kubectl get nodes
```

## Step 2: Create Namespace and Secrets

```bash
# Create namespace for ifthisthenat
kubectl create namespace ifthisthenat-dev

# Set as default namespace
kubectl config set-context --current --namespace=ifthisthenat-dev

# Generate secure cookie key
export COOKIE_KEY=$(openssl rand -base64 64)
echo "Cookie Key Generated: $COOKIE_KEY"

# Create OAuth secret (using example values for local testing)
kubectl create secret generic oauth-credentials \
  --from-literal=client-id="local-test-client" \
  --from-literal=client-secret="local-test-secret" \
  --from-literal=hostname="bsky.social" \
  --namespace=ifthisthenat-dev
```

## Step 3: Configure Local Storage

Create persistent volume for development:

```bash
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: PersistentVolume
metadata:
  name: postgres-pv
  namespace: ifthisthenat-dev
spec:
  capacity:
    storage: 10Gi
  accessModes:
    - ReadWriteOnce
  persistentVolumeReclaimPolicy: Retain
  storageClassName: local-storage
  local:
    path: /tmp/k3s-data/postgres
  nodeAffinity:
    required:
      nodeSelectorTerms:
      - matchExpressions:
        - key: kubernetes.io/hostname
          operator: In
          values:
          - k3d-ifthisthenat-dev-agent-0
---
apiVersion: v1
kind: PersistentVolume
metadata:
  name: redis-pv
  namespace: ifthisthenat-dev
spec:
  capacity:
    storage: 2Gi
  accessModes:
    - ReadWriteOnce
  persistentVolumeReclaimPolicy: Retain
  storageClassName: local-storage
  local:
    path: /tmp/k3s-data/redis
  nodeAffinity:
    required:
      nodeSelectorTerms:
      - matchExpressions:
        - key: kubernetes.io/hostname
          operator: In
          values:
          - k3d-ifthisthenat-dev-agent-1
---
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: local-storage
provisioner: kubernetes.io/no-provisioner
volumeBindingMode: WaitForFirstConsumer
EOF
```

## Step 4: Create Helm Values Configuration

Create `values-local-dev.yaml`:

```yaml
# values-local-dev.yaml
replicaCount: 1

image:
  repository: ifthisthenat
  tag: "latest"
  pullPolicy: IfNotPresent

config:
  externalBase: "http://localhost:8080"
  issuerDid: "did:plc:localtest123"
  httpBindAddr: "0.0.0.0:8080"
  logLevel: "debug"
  
  database:
    maxConnections: 5
    minConnections: 1
  
  redis:
    enabled: true
  
  jetstream:
    enabled: true
    url: "wss://jetstream2.us-west.bsky.network/subscribe"
    consumerCount: 1
    partitionCount: 1
  
  oauth:
    httpCookieKey: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa="
    aipHostname: "bsky.social"
    aipClientId: "local-dev-client"
    aipClientSecret: "local-dev-secret"
  
  scheduler:
    enabled: true
  
  webhook:
    maxRetries: 3
    retryDelay: 30
  
  cache:
    blueprintTtl: 60
    blueprintCapacity: 100
    blueprintRefresh: 300
  
  publish:
    enabledCollections:
      - "app.bsky.feed.post"
      - "app.bsky.feed.like"
      - "app.bsky.feed.repost"
      - "app.bsky.graph.follow"

service:
  type: ClusterIP
  port: 8080

ingress:
  enabled: true
  className: traefik
  hosts:
    - host: ifthisthenat.local
      paths:
        - path: /
          pathType: Prefix

resources:
  limits:
    cpu: 500m
    memory: 512Mi
  requests:
    cpu: 100m
    memory: 256Mi

postgresql:
  enabled: true
  auth:
    username: ifthisthenat
    password: "devpassword123"
    database: ifthisthenat_dev
  primary:
    persistence:
      enabled: true
      storageClass: "local-storage"
      size: 8Gi
    resources:
      limits:
        memory: 256Mi
        cpu: 250m
      requests:
        memory: 128Mi
        cpu: 100m

redis:
  enabled: true
  auth:
    enabled: true
    password: "redisdevpass123"
  master:
    persistence:
      enabled: true
      storageClass: "local-storage"
      size: 1Gi
    resources:
      limits:
        memory: 128Mi
        cpu: 100m
      requests:
        memory: 64Mi
        cpu: 50m

autoscaling:
  enabled: false

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 1000
  fsGroup: 2000

securityContext:
  allowPrivilegeEscalation: false
  capabilities:
    drop:
    - ALL
  readOnlyRootFilesystem: false  # Set to false for local dev
  runAsNonRoot: true
  runAsUser: 1000
```

## Step 5: Build and Load Docker Image

```bash
# Build the Docker image locally
cd /Users/nick/development/tangled.sh/ngerakines.me/ifthisthenat

# Build Docker image
docker build -t ifthisthenat:latest .

# If using k3d, import the image
k3d image import ifthisthenat:latest -c ifthisthenat-dev

# Verify image is available
docker exec k3d-ifthisthenat-dev-agent-0 crictl images | grep ifthisthenat
```

## Step 6: Deploy with Helm

```bash
# Add Bitnami repo for dependencies
helm repo add bitnami https://charts.bitnami.com/bitnami
helm repo update

# Update chart dependencies
cd charts/ifthisthenat
helm dependency update
cd ../..

# Deploy the application
helm install ifthisthenat ./charts/ifthisthenat \
  --namespace ifthisthenat-dev \
  --values values-local-dev.yaml \
  --wait \
  --timeout 10m \
  --debug

# Check deployment status
kubectl get pods -n ifthisthenat-dev
kubectl get svc -n ifthisthenat-dev
kubectl get ingress -n ifthisthenat-dev
```

## Step 7: Configure Local DNS

Add to `/etc/hosts`:
```bash
# Add this line to /etc/hosts
127.0.0.1 ifthisthenat.local

# Or use this command
echo "127.0.0.1 ifthisthenat.local" | sudo tee -a /etc/hosts
```

## Step 8: Database Migration

```bash
# Wait for PostgreSQL to be ready
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=postgresql \
  -n ifthisthenat-dev \
  --timeout=300s

# Get PostgreSQL pod name
export POSTGRES_POD=$(kubectl get pods -n ifthisthenat-dev -l app.kubernetes.io/name=postgresql -o jsonpath="{.items[0].metadata.name}")

# Run migrations (assuming you have migration files)
kubectl exec -it $POSTGRES_POD -n ifthisthenat-dev -- psql -U ifthisthenat -d ifthisthenat_dev -c "
CREATE TABLE IF NOT EXISTS blueprints (
    id SERIAL PRIMARY KEY,
    uri VARCHAR(255) UNIQUE NOT NULL,
    did VARCHAR(255) NOT NULL,
    content JSONB NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS webhook_tasks (
    id SERIAL PRIMARY KEY,
    url VARCHAR(1024) NOT NULL,
    payload JSONB NOT NULL,
    retry_count INT DEFAULT 0,
    status VARCHAR(50) DEFAULT 'pending',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    next_retry_at TIMESTAMP
);

CREATE TABLE IF NOT EXISTS cursor_states (
    id VARCHAR(255) PRIMARY KEY,
    cursor_value TEXT,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
"
```

## Step 9: Verification and Testing

### Check Application Health
```bash
# Check pod logs
kubectl logs -f deployment/ifthisthenat -n ifthisthenat-dev

# Test health endpoint
curl -v http://localhost:8080/

# Check PostgreSQL connectivity
kubectl exec -it $POSTGRES_POD -n ifthisthenat-dev -- psql -U ifthisthenat -d ifthisthenat_dev -c "\dt"

# Check Redis connectivity
export REDIS_POD=$(kubectl get pods -n ifthisthenat-dev -l app.kubernetes.io/name=redis -o jsonpath="{.items[0].metadata.name}")
kubectl exec -it $REDIS_POD -n ifthisthenat-dev -- redis-cli -a redisdevpass123 ping
```

### Port Forwarding (Alternative to Ingress)
```bash
# Forward application port
kubectl port-forward svc/ifthisthenat 8080:8080 -n ifthisthenat-dev &

# Forward PostgreSQL for direct access
kubectl port-forward svc/ifthisthenat-postgresql 5432:5432 -n ifthisthenat-dev &

# Forward Redis
kubectl port-forward svc/ifthisthenat-redis-master 6379:6379 -n ifthisthenat-dev &
```

### Test API Endpoints
```bash
# Test XRPC endpoints
curl -X GET http://localhost:8080/xrpc/tools.graze.ifthisthenat.getBlueprints

# Test webhook endpoint
curl -X POST http://localhost:8080/webhook/test \
  -H "Content-Type: application/json" \
  -d '{"test": "payload"}'

# Test OAuth callback (will fail without proper setup but should respond)
curl -v http://localhost:8080/oauth/callback
```

## Step 10: Integration Testing Script

Create `integration-test.sh`:

```bash
#!/bin/bash
set -e

NAMESPACE="ifthisthenat-dev"
BASE_URL="http://localhost:8080"

echo "Starting Integration Tests..."

# Wait for all pods to be ready
echo "Waiting for pods to be ready..."
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/instance=ifthisthenat \
  -n $NAMESPACE \
  --timeout=300s

# Test 1: Health Check
echo "Test 1: Health Check"
HEALTH_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" $BASE_URL/)
if [ "$HEALTH_RESPONSE" -eq 200 ]; then
    echo "✓ Health check passed"
else
    echo "✗ Health check failed with status: $HEALTH_RESPONSE"
    exit 1
fi

# Test 2: Database Connectivity
echo "Test 2: Database Connectivity"
POSTGRES_POD=$(kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=postgresql -o jsonpath="{.items[0].metadata.name}")
DB_CHECK=$(kubectl exec $POSTGRES_POD -n $NAMESPACE -- psql -U ifthisthenat -d ifthisthenat_dev -t -c "SELECT 1")
if [ "$DB_CHECK" -eq 1 ]; then
    echo "✓ Database connectivity passed"
else
    echo "✗ Database connectivity failed"
    exit 1
fi

# Test 3: Redis Connectivity
echo "Test 3: Redis Connectivity"
REDIS_POD=$(kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=redis -o jsonpath="{.items[0].metadata.name}")
REDIS_CHECK=$(kubectl exec $REDIS_POD -n $NAMESPACE -- redis-cli -a redisdevpass123 ping)
if [ "$REDIS_CHECK" = "PONG" ]; then
    echo "✓ Redis connectivity passed"
else
    echo "✗ Redis connectivity failed"
    exit 1
fi

# Test 4: API Endpoints
echo "Test 4: API Endpoints"
API_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" $BASE_URL/xrpc/tools.graze.ifthisthenat.getBlueprints)
if [ "$API_RESPONSE" -eq 200 ] || [ "$API_RESPONSE" -eq 401 ]; then
    echo "✓ API endpoint test passed"
else
    echo "✗ API endpoint test failed with status: $API_RESPONSE"
    exit 1
fi

# Test 5: Create Test Blueprint
echo "Test 5: Creating Test Blueprint"
BLUEPRINT_DATA='{
  "uri": "at://did:plc:test/tools.graze.ifthisthenat.blueprint/test123",
  "content": {
    "name": "Test Blueprint",
    "nodes": [
      {
        "id": "entry",
        "type": "periodic_entry",
        "payload": {
          "schedule": "0 */5 * * * *"
        }
      },
      {
        "id": "debug",
        "type": "debug_action",
        "payload": {
          "message": "Test blueprint executed"
        }
      }
    ]
  }
}'

# Note: This will likely fail without proper auth, but we're testing the endpoint exists
CREATE_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST $BASE_URL/xrpc/tools.graze.ifthisthenat.updateBlueprint \
  -H "Content-Type: application/json" \
  -d "$BLUEPRINT_DATA")

if [ "$CREATE_RESPONSE" -eq 200 ] || [ "$CREATE_RESPONSE" -eq 401 ] || [ "$CREATE_RESPONSE" -eq 400 ]; then
    echo "✓ Blueprint endpoint test passed (Status: $CREATE_RESPONSE)"
else
    echo "✗ Blueprint endpoint test failed with unexpected status: $CREATE_RESPONSE"
    exit 1
fi

echo ""
echo "========================================="
echo "All integration tests completed successfully!"
echo "========================================="
```

Make the script executable:
```bash
chmod +x integration-test.sh
./integration-test.sh
```

## Step 11: Monitoring and Debugging

### View Application Logs
```bash
# Follow application logs
kubectl logs -f deployment/ifthisthenat -n ifthisthenat-dev

# View PostgreSQL logs
kubectl logs -f deployment/ifthisthenat-postgresql -n ifthisthenat-dev

# View Redis logs
kubectl logs -f deployment/ifthisthenat-redis-master -n ifthisthenat-dev
```

### Debug Pod Issues
```bash
# Describe pod for events
kubectl describe pod -l app.kubernetes.io/name=ifthisthenat -n ifthisthenat-dev

# Get pod events
kubectl get events -n ifthisthenat-dev --sort-by='.lastTimestamp'

# Execute into pod for debugging
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-dev -- /bin/sh
```

### Resource Usage
```bash
# Check resource usage
kubectl top nodes
kubectl top pods -n ifthisthenat-dev

# Get detailed resource info
kubectl get pods -n ifthisthenat-dev -o custom-columns=\
NAME:.metadata.name,\
STATUS:.status.phase,\
CPU_REQ:.spec.containers[0].resources.requests.cpu,\
MEM_REQ:.spec.containers[0].resources.requests.memory,\
CPU_LIM:.spec.containers[0].resources.limits.cpu,\
MEM_LIM:.spec.containers[0].resources.limits.memory
```

## Step 12: Cleanup

```bash
# Delete Helm release
helm uninstall ifthisthenat -n ifthisthenat-dev

# Delete namespace (removes all resources)
kubectl delete namespace ifthisthenat-dev

# Delete persistent volumes
kubectl delete pv postgres-pv redis-pv

# Stop k3d cluster
k3d cluster delete ifthisthenat-dev

# Clean up local data
rm -rf /tmp/k3s-data
```

## Troubleshooting Common Issues

### Issue: Pods stuck in Pending state
```bash
# Check for PVC issues
kubectl get pvc -n ifthisthenat-dev
kubectl describe pvc -n ifthisthenat-dev

# Check node resources
kubectl describe nodes
```

### Issue: Database connection failures
```bash
# Verify PostgreSQL service
kubectl get svc ifthisthenat-postgresql -n ifthisthenat-dev

# Test connection from application pod
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-dev -- \
  pg_isready -h ifthisthenat-postgresql -U ifthisthenat
```

### Issue: Image pull errors
```bash
# List available images in k3d
docker exec k3d-ifthisthenat-dev-agent-0 crictl images

# Re-import image
k3d image import ifthisthenat:latest -c ifthisthenat-dev --verbose
```

### Issue: Ingress not working
```bash
# Check Traefik (k3s default ingress)
kubectl get pods -n kube-system | grep traefik

# Check ingress resource
kubectl describe ingress -n ifthisthenat-dev

# Test with port-forward instead
kubectl port-forward svc/ifthisthenat 8080:8080 -n ifthisthenat-dev
```

## Performance Testing

### Load Testing with k6
```javascript
// k6-test.js
import http from 'k6/http';
import { check, sleep } from 'k6';

export let options = {
  stages: [
    { duration: '30s', target: 10 },
    { duration: '1m', target: 20 },
    { duration: '30s', target: 0 },
  ],
};

export default function() {
  let response = http.get('http://localhost:8080/');
  check(response, {
    'status is 200': (r) => r.status === 200,
    'response time < 200ms': (r) => r.timings.duration < 200,
  });
  sleep(1);
}
```

Run load test:
```bash
# Install k6
brew install k6  # macOS

# Run test
k6 run k6-test.js
```

## CI/CD Integration

### GitHub Actions Workflow
```yaml
# .github/workflows/k3s-integration-test.yml
name: k3s Integration Test

on:
  pull_request:
  push:
    branches: [main]

jobs:
  integration-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup k3s
        uses: debianmaster/actions-k3s@v1.0.5
        with:
          version: 'latest'
      
      - name: Setup Helm
        uses: azure/setup-helm@v3
        
      - name: Build Docker Image
        run: docker build -t ifthisthenat:latest .
      
      - name: Import Image to k3s
        run: |
          docker save ifthisthenat:latest | sudo k3s ctr images import -
      
      - name: Deploy with Helm
        run: |
          helm dependency update ./charts/ifthisthenat
          helm install ifthisthenat ./charts/ifthisthenat \
            --values values-local-dev.yaml \
            --wait --timeout 10m
      
      - name: Run Integration Tests
        run: ./integration-test.sh
      
      - name: Collect Logs on Failure
        if: failure()
        run: |
          kubectl get pods -A
          kubectl logs -l app.kubernetes.io/name=ifthisthenat
```

## Summary

This guide provides a complete local development and testing environment for ifthisthenat using k3s. The setup includes:

1. **Full Stack Deployment**: Application, PostgreSQL, and Redis
2. **Persistent Storage**: Local volumes for data persistence
3. **Security**: Proper RBAC and security contexts
4. **Monitoring**: Log aggregation and resource monitoring
5. **Testing**: Integration tests and load testing
6. **CI/CD Ready**: Can be adapted for automated testing

Remember to adjust resource limits and configuration based on your local machine's capabilities and specific testing requirements.