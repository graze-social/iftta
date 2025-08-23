#!/bin/bash
# Automated k3s Setup Script for ifthisthenat Development
# This script sets up a complete local k3s environment with ifthisthenat

set -e

# Configuration
CLUSTER_NAME="ifthisthenat-dev"
NAMESPACE="ifthisthenat-dev"
CHART_PATH="./charts/ifthisthenat"
VALUES_FILE="./docs/k3s-examples/values-local-dev.yaml"
STORAGE_FILE="./docs/k3s-examples/local-storage.yaml"
DB_INIT_FILE="./docs/k3s-examples/init-db.sql"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Helper functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_step() {
    echo -e "\n${BLUE}==>${NC} $1"
}

# Check prerequisites
check_prerequisites() {
    log_step "Checking prerequisites..."
    
    local missing_tools=()
    
    # Check for required tools
    command -v docker >/dev/null 2>&1 || missing_tools+=("docker")
    command -v kubectl >/dev/null 2>&1 || missing_tools+=("kubectl")
    command -v helm >/dev/null 2>&1 || missing_tools+=("helm")
    command -v k3d >/dev/null 2>&1 || missing_tools+=("k3d")
    
    if [ ${#missing_tools[@]} -gt 0 ]; then
        log_error "Missing required tools: ${missing_tools[*]}"
        echo "Please install the missing tools:"
        echo ""
        for tool in "${missing_tools[@]}"; do
            case $tool in
                docker)
                    echo "  Docker: https://docs.docker.com/get-docker/"
                    ;;
                kubectl)
                    echo "  kubectl: brew install kubectl (macOS) or https://kubernetes.io/docs/tasks/tools/"
                    ;;
                helm)
                    echo "  Helm: brew install helm (macOS) or https://helm.sh/docs/intro/install/"
                    ;;
                k3d)
                    echo "  k3d: brew install k3d (macOS) or https://k3d.io/#installation"
                    ;;
            esac
        done
        exit 1
    fi
    
    log_info "All prerequisites satisfied"
}

# Create k3d cluster
create_cluster() {
    log_step "Creating k3d cluster: $CLUSTER_NAME"
    
    # Check if cluster already exists
    if k3d cluster list | grep -q "$CLUSTER_NAME"; then
        log_warning "Cluster $CLUSTER_NAME already exists"
        read -p "Delete and recreate? (y/n): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            log_info "Deleting existing cluster..."
            k3d cluster delete $CLUSTER_NAME
        else
            log_info "Using existing cluster"
            return 0
        fi
    fi
    
    # Create data directories
    log_info "Creating local data directories..."
    mkdir -p /tmp/k3s-data/postgres
    mkdir -p /tmp/k3s-data/redis
    
    # Create k3d cluster
    log_info "Creating new k3d cluster..."
    k3d cluster create $CLUSTER_NAME \
        --servers 1 \
        --agents 2 \
        --port 8080:80@loadbalancer \
        --port 8443:443@loadbalancer \
        --volume /tmp/k3s-data:/tmp/k3s-data@all \
        --k3s-arg "--disable=traefik@server:0" \
        --wait
    
    # Wait for cluster to be ready
    log_info "Waiting for cluster to be ready..."
    kubectl wait --for=condition=Ready nodes --all --timeout=60s
    
    log_info "Cluster created successfully"
}

# Install Traefik ingress controller
install_ingress() {
    log_step "Installing Traefik Ingress Controller"
    
    helm repo add traefik https://helm.traefik.io/traefik
    helm repo update
    
    helm upgrade --install traefik traefik/traefik \
        --namespace kube-system \
        --set ports.web.nodePort=30080 \
        --set ports.websecure.nodePort=30443 \
        --set service.type=NodePort \
        --wait
    
    log_info "Traefik installed successfully"
}

# Setup namespace
setup_namespace() {
    log_step "Setting up namespace: $NAMESPACE"
    
    # Create namespace
    kubectl create namespace $NAMESPACE --dry-run=client -o yaml | kubectl apply -f -
    
    # Set as default namespace
    kubectl config set-context --current --namespace=$NAMESPACE
    
    log_info "Namespace configured"
}

# Create storage resources
setup_storage() {
    log_step "Setting up persistent storage"
    
    # Apply storage configuration
    kubectl apply -f $STORAGE_FILE
    
    # Verify storage class
    kubectl get storageclass local-storage
    
    log_info "Storage resources created"
}

# Build Docker image
build_image() {
    log_step "Building Docker image"
    
    # Check if Dockerfile exists
    if [ ! -f "Dockerfile" ]; then
        log_warning "Dockerfile not found in current directory"
        log_warning "Creating a placeholder image for testing..."
        
        # Create a simple test image
        cat > /tmp/Dockerfile.test <<EOF
FROM nginx:alpine
RUN echo '{"status": "ok"}' > /usr/share/nginx/html/index.html
EXPOSE 8080
CMD ["nginx", "-g", "daemon off;"]
EOF
        docker build -t ifthisthenat:latest -f /tmp/Dockerfile.test /tmp
    else
        log_info "Building from Dockerfile..."
        docker build -t ifthisthenat:latest .
    fi
    
    # Import image to k3d
    log_info "Importing image to k3d cluster..."
    k3d image import ifthisthenat:latest -c $CLUSTER_NAME
    
    log_info "Docker image ready"
}

# Deploy Helm chart
deploy_helm() {
    log_step "Deploying ifthisthenat with Helm"
    
    # Add Bitnami repo for dependencies
    log_info "Adding Helm repositories..."
    helm repo add bitnami https://charts.bitnami.com/bitnami
    helm repo update
    
    # Update chart dependencies
    log_info "Updating chart dependencies..."
    cd $CHART_PATH
    helm dependency update
    cd - > /dev/null
    
    # Generate cookie key if not set
    if [ -z "$HTTP_COOKIE_KEY" ]; then
        export HTTP_COOKIE_KEY=$(openssl rand -base64 64)
        log_info "Generated HTTP cookie key"
    fi
    
    # Deploy chart
    log_info "Installing Helm chart..."
    helm upgrade --install ifthisthenat $CHART_PATH \
        --namespace $NAMESPACE \
        --values $VALUES_FILE \
        --set config.oauth.httpCookieKey="$HTTP_COOKIE_KEY" \
        --wait \
        --timeout 10m \
        --debug
    
    log_info "Helm chart deployed successfully"
}

# Initialize database
init_database() {
    log_step "Initializing database"
    
    # Wait for PostgreSQL to be ready
    log_info "Waiting for PostgreSQL to be ready..."
    kubectl wait --for=condition=ready pod \
        -l app.kubernetes.io/name=postgresql \
        -n $NAMESPACE \
        --timeout=300s
    
    # Get PostgreSQL pod
    local postgres_pod=$(kubectl get pods -n $NAMESPACE \
        -l app.kubernetes.io/name=postgresql \
        -o jsonpath="{.items[0].metadata.name}")
    
    if [ -z "$postgres_pod" ]; then
        log_warning "PostgreSQL pod not found, skipping database initialization"
        return 0
    fi
    
    # Copy and execute init script
    log_info "Running database initialization script..."
    kubectl cp $DB_INIT_FILE $NAMESPACE/$postgres_pod:/tmp/init.sql
    kubectl exec $postgres_pod -n $NAMESPACE -- \
        psql -U ifthisthenat -d ifthisthenat_dev -f /tmp/init.sql
    
    log_info "Database initialized"
}

# Setup local DNS
setup_dns() {
    log_step "Configuring local DNS"
    
    # Check if entry already exists
    if grep -q "ifthisthenat.local" /etc/hosts; then
        log_info "DNS entry already exists"
    else
        log_info "Adding DNS entry to /etc/hosts..."
        echo "127.0.0.1 ifthisthenat.local" | sudo tee -a /etc/hosts
    fi
    
    log_info "Local DNS configured"
}

# Verify deployment
verify_deployment() {
    log_step "Verifying deployment"
    
    # Check pods
    log_info "Checking pod status..."
    kubectl get pods -n $NAMESPACE
    
    # Check services
    log_info "Checking services..."
    kubectl get svc -n $NAMESPACE
    
    # Setup port forwarding for testing
    log_info "Setting up port forwarding..."
    kubectl port-forward svc/ifthisthenat 8080:8080 -n $NAMESPACE >/dev/null 2>&1 &
    local pf_pid=$!
    sleep 5
    
    # Test endpoint
    log_info "Testing application endpoint..."
    local status=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8080/ 2>/dev/null || echo "000")
    
    # Kill port forwarding
    kill $pf_pid 2>/dev/null || true
    
    if [ "$status" = "200" ] || [ "$status" = "404" ]; then
        log_info "Application is responding (HTTP $status)"
    else
        log_warning "Application returned HTTP $status"
    fi
}

# Print access information
print_access_info() {
    log_step "Access Information"
    
    echo ""
    echo "========================================="
    echo "     ifthisthenat k3s Setup Complete     "
    echo "========================================="
    echo ""
    echo "Cluster Name: $CLUSTER_NAME"
    echo "Namespace: $NAMESPACE"
    echo ""
    echo "Access Methods:"
    echo "  1. Port Forward:"
    echo "     kubectl port-forward svc/ifthisthenat 8080:8080 -n $NAMESPACE"
    echo "     Then visit: http://localhost:8080"
    echo ""
    echo "  2. Via Ingress (if configured):"
    echo "     http://ifthisthenat.local:8080"
    echo ""
    echo "Useful Commands:"
    echo "  - View logs: kubectl logs -f deployment/ifthisthenat -n $NAMESPACE"
    echo "  - Get pods: kubectl get pods -n $NAMESPACE"
    echo "  - Shell into app: kubectl exec -it deployment/ifthisthenat -n $NAMESPACE -- /bin/sh"
    echo "  - Run tests: ./docs/k3s-examples/integration-test.sh"
    echo ""
    echo "Cleanup:"
    echo "  - Delete deployment: helm uninstall ifthisthenat -n $NAMESPACE"
    echo "  - Delete cluster: k3d cluster delete $CLUSTER_NAME"
    echo ""
    echo "========================================="
}

# Cleanup function
cleanup() {
    log_step "Cleaning up k3s environment"
    
    read -p "Are you sure you want to delete everything? (y/n): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        log_info "Cleanup cancelled"
        return 0
    fi
    
    # Delete Helm release
    helm uninstall ifthisthenat -n $NAMESPACE 2>/dev/null || true
    
    # Delete namespace
    kubectl delete namespace $NAMESPACE 2>/dev/null || true
    
    # Delete cluster
    k3d cluster delete $CLUSTER_NAME 2>/dev/null || true
    
    # Clean up data directories
    rm -rf /tmp/k3s-data
    
    log_info "Cleanup complete"
}

# Main menu
show_menu() {
    echo ""
    echo "ifthisthenat k3s Setup Script"
    echo "=============================="
    echo "1. Full Setup (recommended for first time)"
    echo "2. Create Cluster Only"
    echo "3. Deploy Application Only"
    echo "4. Run Integration Tests"
    echo "5. Cleanup Everything"
    echo "6. Exit"
    echo ""
    read -p "Select option: " choice
    
    case $choice in
        1)
            check_prerequisites
            create_cluster
            install_ingress
            setup_namespace
            setup_storage
            build_image
            deploy_helm
            init_database
            setup_dns
            verify_deployment
            print_access_info
            ;;
        2)
            check_prerequisites
            create_cluster
            install_ingress
            ;;
        3)
            check_prerequisites
            setup_namespace
            setup_storage
            build_image
            deploy_helm
            init_database
            verify_deployment
            print_access_info
            ;;
        4)
            ./docs/k3s-examples/integration-test.sh
            ;;
        5)
            cleanup
            ;;
        6)
            exit 0
            ;;
        *)
            log_error "Invalid option"
            show_menu
            ;;
    esac
}

# Main execution
if [ "$1" = "--cleanup" ]; then
    cleanup
elif [ "$1" = "--help" ]; then
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --cleanup    Remove all k3s resources"
    echo "  --help       Show this help message"
    echo ""
    echo "Without options, runs interactive menu"
else
    show_menu
fi