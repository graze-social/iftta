# Kubernetes Production Deployment Guide

This guide covers deploying ifthisthenat on Kubernetes for production use, including Helm charts, monitoring, and scaling configurations.

## Table of Contents

- [Quick Start](#quick-start)
- [Prerequisites](#prerequisites)
- [Architecture Overview](#architecture-overview)
- [Helm Chart Configuration](#helm-chart-configuration)
- [Production Deployment](#production-deployment)
- [Security Configuration](#security-configuration)
- [Monitoring and Observability](#monitoring-and-observability)
- [Scaling and Performance](#scaling-and-performance)
- [Backup and Recovery](#backup-and-recovery)
- [Troubleshooting](#troubleshooting)
- [Local Development Setup](#local-development-setup)

## Quick Start

1. **Clone repository and prepare Helm chart:**
   ```bash
   git clone https://github.com/your-org/ifthisthenat.git
   cd ifthisthenat/charts/ifthisthenat
   helm dependency update
   ```

2. **Create production values file:**
   ```bash
   cp values-production.yaml values-prod.yaml
   # Edit values-prod.yaml with your configuration
   ```

3. **Deploy to Kubernetes:**
   ```bash
   helm install ifthisthenat . \
     --namespace ifthisthenat-prod \
     --create-namespace \
     --values values-prod.yaml
   ```

## Prerequisites

### Kubernetes Cluster Requirements
- **Kubernetes**: 1.19+ (tested with 1.25+)
- **Resources**: 4 CPU cores, 8GB RAM minimum
- **Storage**: 100GB+ with backup capabilities
- **Networking**: Ingress controller (nginx, Traefik, or similar)

### Required Tools
- `kubectl` (matching cluster version)
- `helm` (3.8+)
- `docker` (for building images)

### External Dependencies
- **PostgreSQL**: Can use managed service (RDS, CloudSQL) or in-cluster
- **Redis**: Can use managed service (ElastiCache, MemoryStore) or in-cluster
- **SSL Certificates**: Let's Encrypt, cert-manager, or custom certificates
- **DNS**: External DNS management for ingress

## Architecture Overview

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│  Ingress (HTTPS)│───▶│   ifthisthenat   │───▶│   PostgreSQL    │
│  (nginx/traefik)│    │   Deployment     │    │   StatefulSet   │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                │
                       ┌──────────────────┐
                       │      Redis       │
                       │   StatefulSet    │
                       └──────────────────┘
```

### Kubernetes Resources
- **Deployment**: ifthisthenat application with horizontal pod autoscaler
- **StatefulSet**: PostgreSQL database with persistent volumes
- **StatefulSet**: Redis cache with persistent volumes
- **Services**: ClusterIP services for internal communication
- **Ingress**: HTTPS termination and routing
- **ConfigMaps**: Application configuration
- **Secrets**: Sensitive configuration data
- **PersistentVolumes**: Database and cache storage

## Helm Chart Configuration

### Production Values File

Create `values-production.yaml`:

```yaml
# Production Helm values for ifthisthenat
replicaCount: 3

image:
  repository: your-registry.com/ifthisthenat
  tag: "v1.0.0"
  pullPolicy: IfNotPresent

imagePullSecrets:
  - name: regcred

config:
  externalBase: "https://ifthisthenat.your-domain.com"
  issuerDid: "did:plc:your-production-did"
  httpBindAddr: "0.0.0.0:8080"
  logLevel: "info"

  database:
    maxConnections: 20
    minConnections: 5

  redis:
    enabled: true

  jetstream:
    enabled: true
    url: "wss://jetstream2.us-east.bsky.network/subscribe"
    consumerCount: 2
    partitionCount: 3
    instanceId: 0
    totalInstances: 3

  oauth:
    httpCookieKey: ""  # Set via secret
    aipHostname: "bsky.social"
    aipClientId: ""    # Set via secret
    aipClientSecret: "" # Set via secret
    aipOauthScope: "openid email profile atproto account:email repo:*"

  scheduler:
    enabled: true
    checkInterval: 10
    maxConcurrent: 10

  webhook:
    queueEnabled: true
    maxConcurrent: 20
    maxRetries: 5
    retryDelay: 2000
    queueSize: 5000

  cache:
    blueprintTtl: 300
    blueprintCapacity: 10000
    blueprintRefresh: 60
    identitySize: 50000
    identityTtl: 30

  publish:
    enabledCollections:
      - "app.bsky.feed.post"
      - "app.bsky.feed.like"
      - "app.bsky.feed.repost"
      - "app.bsky.graph.follow"

  security:
    adminDids:
      - "did:plc:admin-1"
      - "did:plc:admin-2"
    disabledNodeTypes: []
    httpClientTimeout: 30

service:
  type: ClusterIP
  port: 8080

ingress:
  enabled: true
  className: "nginx"
  annotations:
    cert-manager.io/cluster-issuer: "letsencrypt-prod"
    nginx.ingress.kubernetes.io/ssl-redirect: "true"
    nginx.ingress.kubernetes.io/force-ssl-redirect: "true"
  hosts:
    - host: ifthisthenat.your-domain.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: ifthisthenat-tls
      hosts:
        - ifthisthenat.your-domain.com

resources:
  limits:
    cpu: 2000m
    memory: 4Gi
  requests:
    cpu: 500m
    memory: 1Gi

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70
  targetMemoryUtilizationPercentage: 80

postgresql:
  enabled: true
  auth:
    username: ifthisthenat
    password: ""  # Set via secret
    database: ifthisthenat_prod
  primary:
    persistence:
      enabled: true
      storageClass: "fast-ssd"
      size: 100Gi
    resources:
      limits:
        memory: 2Gi
        cpu: 1000m
      requests:
        memory: 1Gi
        cpu: 500m
    pgHbaConfiguration: |
      local all all trust
      host all all 127.0.0.1/32 md5
      host all all ::1/128 md5
      host all all 10.0.0.0/8 md5

redis:
  enabled: true
  auth:
    enabled: true
    password: ""  # Set via secret
  master:
    persistence:
      enabled: true
      storageClass: "fast-ssd"
      size: 10Gi
    resources:
      limits:
        memory: 1Gi
        cpu: 500m
      requests:
        memory: 512Mi
        cpu: 100m

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 65534
  runAsGroup: 65534
  fsGroup: 65534

securityContext:
  allowPrivilegeEscalation: false
  capabilities:
    drop:
    - ALL
  readOnlyRootFilesystem: true
  runAsNonRoot: true
  runAsUser: 65534

nodeSelector: {}

tolerations: []

affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
    - weight: 100
      podAffinityTerm:
        labelSelector:
          matchExpressions:
          - key: app.kubernetes.io/name
            operator: In
            values:
            - ifthisthenat
        topologyKey: kubernetes.io/hostname

serviceMonitor:
  enabled: true
  namespace: monitoring
  labels:
    release: prometheus
```

### Creating Secrets

Create required secrets before deployment:

```bash
# Create namespace
kubectl create namespace ifthisthenat-prod

# Create OAuth credentials secret
kubectl create secret generic oauth-credentials \
  --from-literal=client-id="your-oauth-client-id" \
  --from-literal=client-secret="your-oauth-client-secret" \
  --from-literal=cookie-key="$(openssl rand -base64 66)" \
  --namespace=ifthisthenat-prod

# Create database credentials
kubectl create secret generic postgres-credentials \
  --from-literal=password="$(openssl rand -base64 32)" \
  --namespace=ifthisthenat-prod

# Create Redis credentials
kubectl create secret generic redis-credentials \
  --from-literal=password="$(openssl rand -base64 32)" \
  --namespace=ifthisthenat-prod

# Create image pull secret (if using private registry)
kubectl create secret docker-registry regcred \
  --docker-server=your-registry.com \
  --docker-username=your-username \
  --docker-password=your-password \
  --docker-email=your-email@company.com \
  --namespace=ifthisthenat-prod
```

## Production Deployment

### 1. Build and Push Docker Image

```bash
# Build production image
docker build -t your-registry.com/ifthisthenat:v1.0.0 .

# Push to registry
docker push your-registry.com/ifthisthenat:v1.0.0
```

### 2. Deploy with Helm

```bash
# Add required Helm repositories
helm repo add bitnami https://charts.bitnami.com/bitnami
helm repo update

# Install the application
helm install ifthisthenat ./charts/ifthisthenat \
  --namespace ifthisthenat-prod \
  --create-namespace \
  --values values-production.yaml \
  --wait \
  --timeout 15m

# Check deployment status
kubectl get pods -n ifthisthenat-prod
kubectl get services -n ifthisthenat-prod
kubectl get ingress -n ifthisthenat-prod
```

### 3. Verify Deployment

```bash
# Check pod logs
kubectl logs -f deployment/ifthisthenat -n ifthisthenat-prod

# Test application health
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-prod -- \
  curl -f http://localhost:8080/health

# Test database connectivity
kubectl exec -it statefulset/ifthisthenat-postgresql -n ifthisthenat-prod -- \
  pg_isready -U ifthisthenat

# Test Redis connectivity
kubectl exec -it statefulset/ifthisthenat-redis-master -n ifthisthenat-prod -- \
  redis-cli ping
```

### 4. Configure DNS

```bash
# Get ingress IP
kubectl get ingress ifthisthenat -n ifthisthenat-prod

# Configure DNS A record
# ifthisthenat.your-domain.com -> INGRESS_IP
```

## Security Configuration

### Network Policies

Create network policies to restrict traffic:

```yaml
# network-policy.yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: ifthisthenat-network-policy
  namespace: ifthisthenat-prod
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: ifthisthenat
  policyTypes:
  - Ingress
  - Egress
  ingress:
  - from:
    - namespaceSelector:
        matchLabels:
          name: ingress-nginx
    ports:
    - protocol: TCP
      port: 8080
  egress:
  - to:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: postgresql
    ports:
    - protocol: TCP
      port: 5432
  - to:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: redis
    ports:
    - protocol: TCP
      port: 6379
  - to: []  # Allow external traffic for webhooks and AT Protocol
    ports:
    - protocol: TCP
      port: 443
    - protocol: TCP
      port: 80
```

Apply the network policy:
```bash
kubectl apply -f network-policy.yaml
```

### Pod Security Standards

Enable pod security standards:

```yaml
# pod-security-policy.yaml
apiVersion: v1
kind: Namespace
metadata:
  name: ifthisthenat-prod
  labels:
    pod-security.kubernetes.io/enforce: restricted
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/warn: restricted
```

### RBAC Configuration

Create service account with minimal permissions:

```yaml
# rbac.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: ifthisthenat
  namespace: ifthisthenat-prod
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  namespace: ifthisthenat-prod
  name: ifthisthenat-role
rules:
- apiGroups: [""]
  resources: ["pods"]
  verbs: ["get", "list"]
- apiGroups: [""]
  resources: ["configmaps"]
  verbs: ["get", "list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: ifthisthenat-rolebinding
  namespace: ifthisthenat-prod
subjects:
- kind: ServiceAccount
  name: ifthisthenat
  namespace: ifthisthenat-prod
roleRef:
  kind: Role
  name: ifthisthenat-role
  apiGroup: rbac.authorization.k8s.io
```

## Monitoring and Observability

### Prometheus Monitoring

Create ServiceMonitor for Prometheus:

```yaml
# service-monitor.yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: ifthisthenat
  namespace: ifthisthenat-prod
  labels:
    app.kubernetes.io/name: ifthisthenat
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: ifthisthenat
  endpoints:
  - port: http
    path: /metrics
    interval: 30s
    scrapeTimeout: 10s
```

### Grafana Dashboard

Create dashboard configuration:

```json
{
  "dashboard": {
    "title": "ifthisthenat Production Metrics",
    "panels": [
      {
        "title": "Request Rate",
        "type": "graph",
        "targets": [
          {
            "expr": "rate(http_requests_total{job=\"ifthisthenat\"}[5m])"
          }
        ]
      },
      {
        "title": "Error Rate",
        "type": "graph",
        "targets": [
          {
            "expr": "rate(http_requests_total{job=\"ifthisthenat\",status=~\"5..\"}[5m])"
          }
        ]
      },
      {
        "title": "Response Time",
        "type": "graph",
        "targets": [
          {
            "expr": "histogram_quantile(0.95, rate(http_request_duration_seconds_bucket{job=\"ifthisthenat\"}[5m]))"
          }
        ]
      },
      {
        "title": "Blueprint Queue Depth",
        "type": "graph",
        "targets": [
          {
            "expr": "blueprint_queue_depth{job=\"ifthisthenat\"}"
          }
        ]
      }
    ]
  }
}
```

### Logging with ELK Stack

Configure logging:

```yaml
# logging-configmap.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: fluent-bit-config
  namespace: ifthisthenat-prod
data:
  fluent-bit.conf: |
    [INPUT]
        Name tail
        Path /var/log/containers/ifthisthenat-*.log
        Parser docker
        Tag kube.*
        Refresh_Interval 5
        Mem_Buf_Limit 50MB

    [OUTPUT]
        Name es
        Match kube.*
        Host elasticsearch.logging.svc.cluster.local
        Port 9200
        Index ifthisthenat-prod
        Type _doc
```

### Health Checks and Alerts

Configure AlertManager rules:

```yaml
# alert-rules.yaml
groups:
- name: ifthisthenat
  rules:
  - alert: ifthisthenatDown
    expr: up{job="ifthisthenat"} == 0
    for: 1m
    labels:
      severity: critical
    annotations:
      summary: "ifthisthenat instance is down"
      description: "ifthisthenat instance has been down for more than 1 minute"

  - alert: HighErrorRate
    expr: rate(http_requests_total{job="ifthisthenat",status=~"5.."}[5m]) > 0.1
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "High error rate detected"
      description: "Error rate is {{ $value }} requests per second"

  - alert: HighMemoryUsage
    expr: container_memory_usage_bytes{pod=~"ifthisthenat-.*"} / container_spec_memory_limit_bytes > 0.9
    for: 10m
    labels:
      severity: warning
    annotations:
      summary: "High memory usage"
      description: "Memory usage is above 90%"
```

## Scaling and Performance

### Horizontal Pod Autoscaler

Configure HPA based on CPU and memory:

```yaml
# hpa.yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: ifthisthenat-hpa
  namespace: ifthisthenat-prod
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: ifthisthenat
  minReplicas: 3
  maxReplicas: 20
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Resource
    resource:
      name: memory
      target:
        type: Utilization
        averageUtilization: 80
  behavior:
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
      - type: Pods
        value: 2
        periodSeconds: 60
    scaleUp:
      stabilizationWindowSeconds: 60
      policies:
      - type: Pods
        value: 4
        periodSeconds: 60
      - type: Percent
        value: 100
        periodSeconds: 15
```

### Vertical Pod Autoscaler

Configure VPA for automatic resource recommendations:

```yaml
# vpa.yaml
apiVersion: autoscaling.k8s.io/v1
kind: VerticalPodAutoscaler
metadata:
  name: ifthisthenat-vpa
  namespace: ifthisthenat-prod
spec:
  targetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: ifthisthenat
  updatePolicy:
    updateMode: "Auto"
  resourcePolicy:
    containerPolicies:
    - containerName: ifthisthenat
      maxAllowed:
        cpu: 4
        memory: 8Gi
      minAllowed:
        cpu: 100m
        memory: 256Mi
```

### Node Affinity and Taints

Configure node affinity for better resource utilization:

```yaml
# In values-production.yaml
affinity:
  nodeAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      nodeSelectorTerms:
      - matchExpressions:
        - key: node-type
          operator: In
          values:
          - "compute-optimized"
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
    - weight: 100
      podAffinityTerm:
        labelSelector:
          matchExpressions:
          - key: app.kubernetes.io/name
            operator: In
            values:
            - ifthisthenat
        topologyKey: kubernetes.io/hostname

tolerations:
- key: "dedicated"
  operator: "Equal"
  value: "ifthisthenat"
  effect: "NoSchedule"
```

## Backup and Recovery

### Database Backup with CronJob

```yaml
# backup-cronjob.yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: postgres-backup
  namespace: ifthisthenat-prod
spec:
  schedule: "0 2 * * *"  # Daily at 2 AM
  jobTemplate:
    spec:
      template:
        spec:
          containers:
          - name: postgres-backup
            image: postgres:15-alpine
            env:
            - name: PGPASSWORD
              valueFrom:
                secretKeyRef:
                  name: postgres-credentials
                  key: password
            command:
            - /bin/sh
            - -c
            - |
              DATE=$(date +%Y%m%d_%H%M%S)
              pg_dump -h ifthisthenat-postgresql -U ifthisthenat ifthisthenat_prod > /backup/backup_$DATE.sql
              # Upload to S3, GCS, or other storage
              aws s3 cp /backup/backup_$DATE.sql s3://your-backup-bucket/postgres/
              # Clean up local backup
              rm /backup/backup_$DATE.sql
            volumeMounts:
            - name: backup-volume
              mountPath: /backup
          volumes:
          - name: backup-volume
            emptyDir: {}
          restartPolicy: OnFailure
  successfulJobsHistoryLimit: 3
  failedJobsHistoryLimit: 1
```

### Velero for Cluster Backup

Install and configure Velero:

```bash
# Install Velero
helm repo add vmware-tanzu https://vmware-tanzu.github.io/helm-charts/
helm install velero vmware-tanzu/velero \
  --namespace velero \
  --create-namespace \
  --set-file credentials.secretContents.cloud=credentials-velero \
  --set configuration.provider=aws \
  --set configuration.backupStorageLocation.bucket=your-backup-bucket \
  --set configuration.backupStorageLocation.config.region=us-west-2

# Create backup schedule
kubectl apply -f - <<EOF
apiVersion: velero.io/v1
kind: Schedule
metadata:
  name: ifthisthenat-backup
  namespace: velero
spec:
  schedule: "0 1 * * *"  # Daily at 1 AM
  template:
    includedNamespaces:
    - ifthisthenat-prod
    ttl: 720h  # 30 days
EOF
```

### Disaster Recovery Plan

Create disaster recovery runbook:

```bash
#!/bin/bash
# disaster-recovery.sh

# 1. Restore from Velero backup
velero restore create --from-backup ifthisthenat-backup-20241201-010000

# 2. Restore database from latest backup
kubectl exec -it statefulset/ifthisthenat-postgresql -n ifthisthenat-prod -- \
  psql -U ifthisthenat -d ifthisthenat_prod < latest_backup.sql

# 3. Verify service health
kubectl get pods -n ifthisthenat-prod
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-prod -- \
  curl -f http://localhost:8080/health

# 4. Update DNS if needed
kubectl get ingress -n ifthisthenat-prod
```

## Troubleshooting

### Common Issues

#### Pods Stuck in Pending State

```bash
# Check resource availability
kubectl describe nodes

# Check PVC status
kubectl get pvc -n ifthisthenat-prod

# Check pod events
kubectl describe pod -l app.kubernetes.io/name=ifthisthenat -n ifthisthenat-prod
```

#### Database Connection Issues

```bash
# Check PostgreSQL service
kubectl get svc ifthisthenat-postgresql -n ifthisthenat-prod

# Test connectivity from app pod
kubectl exec -it deployment/ifthisthenat -n ifthisthenat-prod -- \
  pg_isready -h ifthisthenat-postgresql -U ifthisthenat

# Check PostgreSQL logs
kubectl logs statefulset/ifthisthenat-postgresql -n ifthisthenat-prod
```

#### Image Pull Errors

```bash
# Check image pull secrets
kubectl get secrets -n ifthisthenat-prod | grep regcred

# Test image pull manually
kubectl run test-pod --image=your-registry.com/ifthisthenat:v1.0.0 \
  --dry-run=client -o yaml | kubectl apply -f -
```

#### High Memory/CPU Usage

```bash
# Check resource usage
kubectl top pods -n ifthisthenat-prod

# Check HPA status
kubectl get hpa -n ifthisthenat-prod

# Scale manually if needed
kubectl scale deployment ifthisthenat --replicas=5 -n ifthisthenat-prod
```

### Debug Commands

```bash
# Get comprehensive cluster state
kubectl get all -n ifthisthenat-prod

# Check events
kubectl get events -n ifthisthenat-prod --sort-by='.lastTimestamp'

# Debug pod issues
kubectl describe pod <pod-name> -n ifthisthenat-prod

# Access pod shell
kubectl exec -it <pod-name> -n ifthisthenat-prod -- /bin/sh

# Port forward for direct access
kubectl port-forward svc/ifthisthenat 8080:8080 -n ifthisthenat-prod

# Check logs with context
kubectl logs -f deployment/ifthisthenat -n ifthisthenat-prod --previous
```

## Local Development Setup

For local Kubernetes development, see the [k3s local deployment guide](k3s-local-deployment.md) and use the files in [k3s-examples/](k3s-examples/).

### Quick Local Setup

```bash
# Use the provided setup script
cd docs/k3s-examples
chmod +x setup-k3s.sh
./setup-k3s.sh

# Or use Docker Compose for simpler local testing
docker-compose -f docs/k3s-examples/docker-compose.yaml up -d
```

This Kubernetes deployment guide provides enterprise-grade deployment patterns for ifthisthenat. For simpler deployments, consider the [standalone deployment guide](deployment-standalone.md).