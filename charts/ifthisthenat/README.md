# ifthisthenat Helm Chart

This Helm chart deploys ifthisthenat - an AT Protocol automation service - on a Kubernetes cluster.

## Prerequisites

- Kubernetes 1.19+
- Helm 3.0+
- PostgreSQL database (can be deployed with the chart or external)
- Redis (optional, for caching and queuing)

## Installation

### Add the repository (if published)

```bash
helm repo add ifthisthenat https://charts.example.com
helm repo update
```

### Install from local directory

```bash
# From the charts directory
helm install ifthisthenat ./ifthisthenat -n ifthisthenat --create-namespace

# Or from the project root
helm install ifthisthenat ./charts/ifthisthenat -n ifthisthenat --create-namespace
```

## Configuration

### Required Configuration

Before deployment, you must configure:

1. **Database Connection**
   ```yaml
   config:
     database:
       url: "postgres://user:password@host:5432/database"
   ```

2. **OAuth Configuration**
   ```yaml
   config:
     oauth:
       httpCookieKey: "base64-encoded-key"  # Generate: openssl rand -base64 64
       aipHostname: "your-oauth-provider.com"
       aipClientId: "your-client-id"
       aipClientSecret: "your-client-secret"
   ```

3. **Service Identity**
   ```yaml
   config:
     issuerDid: "did:plc:yourdid"
     externalBase: "https://your-service.com"
   ```

### Example values.yaml

```yaml
image:
  repository: your-registry/ifthisthenat
  tag: "latest"

config:
  externalBase: "https://ifthisthenat.example.com"
  issuerDid: "did:plc:yourdid"
  
  oauth:
    aipHostname: "bsky.social"
    aipClientId: "your-client-id"
    aipClientSecret: "your-secret"
    httpCookieKey: "your-base64-encoded-key"
  
  database:
    url: "postgres://ifthisthenat:password@postgres:5432/ifthisthenat"
  
  redis:
    enabled: true
    url: "redis://:password@redis:6379/0"
  
  jetstream:
    enabled: true

ingress:
  enabled: true
  className: nginx
  hosts:
    - host: ifthisthenat.example.com
      paths:
        - path: /
          pathType: ImplementationSpecific
  tls:
    - secretName: ifthisthenat-tls
      hosts:
        - ifthisthenat.example.com

resources:
  requests:
    memory: "256Mi"
    cpu: "250m"
  limits:
    memory: "512Mi"
    cpu: "500m"
```

## Installing with PostgreSQL

The chart can deploy PostgreSQL automatically:

```bash
helm install ifthisthenat ./ifthisthenat \
  --set postgresql.enabled=true \
  --set postgresql.auth.password=secretpassword \
  --set config.oauth.httpCookieKey=$(openssl rand -base64 64) \
  --set config.issuerDid=did:plc:yourdid
```

## Installing with Redis

Enable Redis for caching and queue support:

```bash
helm install ifthisthenat ./ifthisthenat \
  --set redis.enabled=true \
  --set redis.auth.password=redispassword
```

## Upgrading

```bash
helm upgrade ifthisthenat ./ifthisthenat -n ifthisthenat
```

## Uninstallation

```bash
helm uninstall ifthisthenat -n ifthisthenat
```

## Parameters

### Image Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `image.repository` | Image repository | `ifthisthenat` |
| `image.tag` | Image tag | `""` (uses chart appVersion) |
| `image.pullPolicy` | Image pull policy | `IfNotPresent` |

### Configuration Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `config.externalBase` | External base URL | `http://localhost:8080` |
| `config.issuerDid` | Service DID | `""` |
| `config.database.url` | PostgreSQL connection URL | `""` |
| `config.redis.enabled` | Enable Redis | `false` |
| `config.redis.url` | Redis connection URL | `""` |
| `config.jetstream.enabled` | Enable Jetstream consumer | `false` |

### Service Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `service.type` | Service type | `ClusterIP` |
| `service.port` | Service port | `8080` |

### Ingress Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.enabled` | Enable ingress | `false` |
| `ingress.className` | Ingress class name | `""` |
| `ingress.hosts` | Ingress hosts | See values.yaml |
| `ingress.tls` | TLS configuration | `[]` |

### Resource Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `resources.limits.cpu` | CPU limit | `1000m` |
| `resources.limits.memory` | Memory limit | `512Mi` |
| `resources.requests.cpu` | CPU request | `100m` |
| `resources.requests.memory` | Memory request | `128Mi` |

### Autoscaling Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `autoscaling.enabled` | Enable HPA | `false` |
| `autoscaling.minReplicas` | Minimum replicas | `1` |
| `autoscaling.maxReplicas` | Maximum replicas | `10` |
| `autoscaling.targetCPUUtilizationPercentage` | Target CPU utilization | `80` |

## Troubleshooting

### Check pod status
```bash
kubectl get pods -n ifthisthenat
```

### View logs
```bash
kubectl logs -f deployment/ifthisthenat-ifthisthenat -n ifthisthenat
```

### Database connectivity issues
Ensure the database URL is correct and the database is accessible from the cluster.

### OAuth issues
Verify that the OAuth credentials are correct and the cookie key is properly base64 encoded.