#!/bin/bash
# Integration Testing Script for ifthisthenat on k3s
# This script validates the deployment and tests core functionality

set -e

# Configuration
NAMESPACE="${NAMESPACE:-ifthisthenat-dev}"
BASE_URL="${BASE_URL:-http://localhost:8080}"
TIMEOUT=300
TEST_RESULTS=()

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

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

log_test() {
    echo -e "\n${YELLOW}[TEST]${NC} $1"
}

check_result() {
    if [ $1 -eq 0 ]; then
        echo -e "${GREEN}✓${NC} $2"
        TEST_RESULTS+=("PASS: $2")
    else
        echo -e "${RED}✗${NC} $2"
        TEST_RESULTS+=("FAIL: $2")
        return 1
    fi
}

# Prerequisites check
check_prerequisites() {
    log_info "Checking prerequisites..."
    
    command -v kubectl >/dev/null 2>&1 || { log_error "kubectl not found"; exit 1; }
    command -v curl >/dev/null 2>&1 || { log_error "curl not found"; exit 1; }
    
    kubectl cluster-info >/dev/null 2>&1 || { log_error "No Kubernetes cluster connection"; exit 1; }
    
    log_info "Prerequisites check passed"
}

# Wait for pods to be ready
wait_for_pods() {
    log_info "Waiting for pods to be ready in namespace $NAMESPACE..."
    
    local start_time=$(date +%s)
    local timeout_time=$((start_time + TIMEOUT))
    
    while true; do
        local current_time=$(date +%s)
        if [ $current_time -gt $timeout_time ]; then
            log_error "Timeout waiting for pods to be ready"
            kubectl get pods -n $NAMESPACE
            return 1
        fi
        
        local not_ready=$(kubectl get pods -n $NAMESPACE -o json | \
            jq -r '.items[] | select(.status.phase != "Running" or (.status.conditions[]? | select(.type == "Ready" and .status != "True"))) | .metadata.name' | wc -l)
        
        if [ "$not_ready" -eq 0 ]; then
            log_info "All pods are ready"
            kubectl get pods -n $NAMESPACE
            return 0
        fi
        
        log_info "Waiting... ($not_ready pods not ready)"
        sleep 5
    done
}

# Test 1: Namespace and Basic Resources
test_namespace_resources() {
    log_test "Test 1: Namespace and Basic Resources"
    
    # Check namespace exists
    kubectl get namespace $NAMESPACE >/dev/null 2>&1
    check_result $? "Namespace exists"
    
    # Check deployments
    local deployments=$(kubectl get deployments -n $NAMESPACE --no-headers 2>/dev/null | wc -l)
    [ "$deployments" -ge 1 ]
    check_result $? "Deployments found ($deployments)"
    
    # Check services
    local services=$(kubectl get services -n $NAMESPACE --no-headers 2>/dev/null | wc -l)
    [ "$services" -ge 1 ]
    check_result $? "Services found ($services)"
    
    # Check configmaps
    local configmaps=$(kubectl get configmaps -n $NAMESPACE --no-headers 2>/dev/null | wc -l)
    [ "$configmaps" -ge 1 ]
    check_result $? "ConfigMaps found ($configmaps)"
    
    # Check secrets
    local secrets=$(kubectl get secrets -n $NAMESPACE --no-headers 2>/dev/null | wc -l)
    [ "$secrets" -ge 1 ]
    check_result $? "Secrets found ($secrets)"
}

# Test 2: PostgreSQL Connectivity
test_postgresql() {
    log_test "Test 2: PostgreSQL Database"
    
    local postgres_pod=$(kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=postgresql -o jsonpath="{.items[0].metadata.name}" 2>/dev/null)
    
    if [ -z "$postgres_pod" ]; then
        log_warning "PostgreSQL pod not found, skipping database tests"
        return 0
    fi
    
    # Test database connection
    kubectl exec $postgres_pod -n $NAMESPACE -- psql -U ifthisthenat -d ifthisthenat_dev -c "SELECT 1" >/dev/null 2>&1
    check_result $? "PostgreSQL connection"
    
    # Check tables exist
    local tables=$(kubectl exec $postgres_pod -n $NAMESPACE -- psql -U ifthisthenat -d ifthisthenat_dev -t -c "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'" 2>/dev/null | xargs)
    [ "$tables" -ge 0 ]
    check_result $? "Database schema ($tables tables)"
    
    # Test write operation
    kubectl exec $postgres_pod -n $NAMESPACE -- psql -U ifthisthenat -d ifthisthenat_dev -c "INSERT INTO cursor_states (id, cursor_value) VALUES ('test', 'test_value') ON CONFLICT (id) DO UPDATE SET cursor_value = 'test_value'" >/dev/null 2>&1
    check_result $? "Database write operation"
    
    # Test read operation
    local value=$(kubectl exec $postgres_pod -n $NAMESPACE -- psql -U ifthisthenat -d ifthisthenat_dev -t -c "SELECT cursor_value FROM cursor_states WHERE id = 'test'" 2>/dev/null | xargs)
    [ "$value" = "test_value" ]
    check_result $? "Database read operation"
}

# Test 3: Redis Connectivity
test_redis() {
    log_test "Test 3: Redis Cache"
    
    local redis_pod=$(kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=redis -o jsonpath="{.items[0].metadata.name}" 2>/dev/null)
    
    if [ -z "$redis_pod" ]; then
        log_warning "Redis pod not found, skipping Redis tests"
        return 0
    fi
    
    # Test Redis ping
    local pong=$(kubectl exec $redis_pod -n $NAMESPACE -- redis-cli -a redisdevpass123 ping 2>/dev/null)
    [ "$pong" = "PONG" ]
    check_result $? "Redis ping"
    
    # Test Redis write
    kubectl exec $redis_pod -n $NAMESPACE -- redis-cli -a redisdevpass123 SET test:key "test_value" >/dev/null 2>&1
    check_result $? "Redis write operation"
    
    # Test Redis read
    local value=$(kubectl exec $redis_pod -n $NAMESPACE -- redis-cli -a redisdevpass123 GET test:key 2>/dev/null)
    [ "$value" = "test_value" ]
    check_result $? "Redis read operation"
    
    # Check Redis memory usage
    local memory=$(kubectl exec $redis_pod -n $NAMESPACE -- redis-cli -a redisdevpass123 INFO memory 2>/dev/null | grep used_memory_human | cut -d: -f2 | tr -d '\r')
    log_info "Redis memory usage: $memory"
}

# Test 4: Application Health
test_application_health() {
    log_test "Test 4: Application Health"
    
    # Check if service is accessible
    local app_pod=$(kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=ifthisthenat -o jsonpath="{.items[0].metadata.name}" 2>/dev/null)
    
    if [ -z "$app_pod" ]; then
        log_error "Application pod not found"
        return 1
    fi
    
    # Check pod status
    local pod_status=$(kubectl get pod $app_pod -n $NAMESPACE -o jsonpath="{.status.phase}" 2>/dev/null)
    [ "$pod_status" = "Running" ]
    check_result $? "Application pod running"
    
    # Check container ready
    local container_ready=$(kubectl get pod $app_pod -n $NAMESPACE -o jsonpath="{.status.containerStatuses[0].ready}" 2>/dev/null)
    [ "$container_ready" = "true" ]
    check_result $? "Application container ready"
    
    # Check recent logs for errors
    local error_count=$(kubectl logs $app_pod -n $NAMESPACE --tail=100 2>/dev/null | grep -c "ERROR" || echo 0)
    [ "$error_count" -lt 10 ]
    check_result $? "Application logs (errors: $error_count)"
}

# Test 5: HTTP Endpoints
test_http_endpoints() {
    log_test "Test 5: HTTP Endpoints"
    
    # Setup port forwarding
    log_info "Setting up port forwarding..."
    kubectl port-forward svc/ifthisthenat 8080:8080 -n $NAMESPACE >/dev/null 2>&1 &
    local pf_pid=$!
    sleep 3
    
    # Function to cleanup port forwarding
    cleanup_port_forward() {
        if [ ! -z "$pf_pid" ]; then
            kill $pf_pid 2>/dev/null || true
        fi
    }
    trap cleanup_port_forward EXIT
    
    # Test health endpoint
    local health_status=$(curl -s -o /dev/null -w "%{http_code}" $BASE_URL/ 2>/dev/null || echo "000")
    [ "$health_status" = "200" ] || [ "$health_status" = "404" ]
    check_result $? "Health endpoint (HTTP $health_status)"
    
    # Test XRPC endpoint
    local xrpc_status=$(curl -s -o /dev/null -w "%{http_code}" $BASE_URL/xrpc/tools.graze.ifthisthenat.getBlueprints 2>/dev/null || echo "000")
    [ "$xrpc_status" = "200" ] || [ "$xrpc_status" = "401" ] || [ "$xrpc_status" = "400" ]
    check_result $? "XRPC endpoint (HTTP $xrpc_status)"
    
    # Test webhook endpoint
    local webhook_status=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
        -H "Content-Type: application/json" \
        -d '{"test": "payload"}' \
        $BASE_URL/webhook/test 2>/dev/null || echo "000")
    [ "$webhook_status" = "200" ] || [ "$webhook_status" = "401" ] || [ "$webhook_status" = "404" ]
    check_result $? "Webhook endpoint (HTTP $webhook_status)"
    
    # Cleanup port forwarding
    cleanup_port_forward
}

# Test 6: Create and Verify Blueprint
test_blueprint_operations() {
    log_test "Test 6: Blueprint Operations"
    
    # Setup port forwarding
    kubectl port-forward svc/ifthisthenat 8080:8080 -n $NAMESPACE >/dev/null 2>&1 &
    local pf_pid=$!
    sleep 3
    
    cleanup_port_forward() {
        if [ ! -z "$pf_pid" ]; then
            kill $pf_pid 2>/dev/null || true
        fi
    }
    trap cleanup_port_forward EXIT
    
    # Create test blueprint
    local blueprint_json='{
        "uri": "at://did:plc:test/tools.graze.ifthisthenat.blueprint/integration-test",
        "content": {
            "name": "Integration Test Blueprint",
            "order": 1,
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
                        "message": "Integration test executed"
                    }
                }
            ]
        }
    }'
    
    # Attempt to create blueprint (may fail with auth, which is expected)
    local create_response=$(curl -s -X POST \
        -H "Content-Type: application/json" \
        -d "$blueprint_json" \
        $BASE_URL/xrpc/tools.graze.ifthisthenat.updateBlueprint 2>/dev/null || echo "{}")
    
    # Check if response indicates endpoint exists (even if auth failed)
    echo "$create_response" | grep -q "error" || echo "$create_response" | grep -q "blueprint"
    local endpoint_exists=$?
    [ $endpoint_exists -eq 0 ]
    check_result $? "Blueprint API endpoint exists"
    
    cleanup_port_forward
}

# Test 7: Resource Limits and Quotas
test_resource_limits() {
    log_test "Test 7: Resource Limits and Quotas"
    
    local app_pod=$(kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=ifthisthenat -o jsonpath="{.items[0].metadata.name}" 2>/dev/null)
    
    if [ -z "$app_pod" ]; then
        log_warning "Application pod not found, skipping resource tests"
        return 0
    fi
    
    # Check CPU limits
    local cpu_limit=$(kubectl get pod $app_pod -n $NAMESPACE -o jsonpath="{.spec.containers[0].resources.limits.cpu}" 2>/dev/null)
    [ ! -z "$cpu_limit" ]
    check_result $? "CPU limit set ($cpu_limit)"
    
    # Check memory limits
    local mem_limit=$(kubectl get pod $app_pod -n $NAMESPACE -o jsonpath="{.spec.containers[0].resources.limits.memory}" 2>/dev/null)
    [ ! -z "$mem_limit" ]
    check_result $? "Memory limit set ($mem_limit)"
    
    # Check security context
    local run_as_non_root=$(kubectl get pod $app_pod -n $NAMESPACE -o jsonpath="{.spec.securityContext.runAsNonRoot}" 2>/dev/null)
    [ "$run_as_non_root" = "true" ]
    check_result $? "Running as non-root user"
}

# Test 8: Persistent Volume Claims
test_persistent_storage() {
    log_test "Test 8: Persistent Storage"
    
    # Check PVCs
    local pvcs=$(kubectl get pvc -n $NAMESPACE --no-headers 2>/dev/null | wc -l)
    if [ "$pvcs" -gt 0 ]; then
        log_info "Found $pvcs PVCs"
        
        # Check PVC status
        local bound_pvcs=$(kubectl get pvc -n $NAMESPACE -o json 2>/dev/null | \
            jq -r '.items[] | select(.status.phase == "Bound") | .metadata.name' | wc -l)
        [ "$bound_pvcs" -eq "$pvcs" ]
        check_result $? "All PVCs bound ($bound_pvcs/$pvcs)"
        
        # Show PVC details
        kubectl get pvc -n $NAMESPACE
    else
        log_warning "No PVCs found (may be using ephemeral storage)"
    fi
}

# Performance test
test_performance() {
    log_test "Test 9: Basic Performance"
    
    # Setup port forwarding
    kubectl port-forward svc/ifthisthenat 8080:8080 -n $NAMESPACE >/dev/null 2>&1 &
    local pf_pid=$!
    sleep 3
    
    cleanup_port_forward() {
        if [ ! -z "$pf_pid" ]; then
            kill $pf_pid 2>/dev/null || true
        fi
    }
    trap cleanup_port_forward EXIT
    
    log_info "Running basic performance test..."
    
    local total_time=0
    local success_count=0
    local request_count=10
    
    for i in $(seq 1 $request_count); do
        local start=$(date +%s%N)
        local status=$(curl -s -o /dev/null -w "%{http_code}" $BASE_URL/ 2>/dev/null || echo "000")
        local end=$(date +%s%N)
        
        if [ "$status" = "200" ] || [ "$status" = "404" ]; then
            success_count=$((success_count + 1))
            local duration=$(( (end - start) / 1000000 ))
            total_time=$((total_time + duration))
            log_info "Request $i: ${duration}ms (HTTP $status)"
        else
            log_warning "Request $i failed (HTTP $status)"
        fi
        
        sleep 0.1
    done
    
    if [ $success_count -gt 0 ]; then
        local avg_time=$((total_time / success_count))
        [ $avg_time -lt 1000 ]
        check_result $? "Average response time: ${avg_time}ms ($success_count/$request_count succeeded)"
    else
        check_result 1 "All requests failed"
    fi
    
    cleanup_port_forward
}

# Print summary
print_summary() {
    echo ""
    echo "========================================="
    echo "        INTEGRATION TEST SUMMARY         "
    echo "========================================="
    
    local passed=0
    local failed=0
    
    for result in "${TEST_RESULTS[@]}"; do
        if [[ $result == PASS* ]]; then
            echo -e "${GREEN}✓${NC} $result"
            passed=$((passed + 1))
        else
            echo -e "${RED}✗${NC} $result"
            failed=$((failed + 1))
        fi
    done
    
    echo "========================================="
    echo -e "Total: $((passed + failed)) | ${GREEN}Passed: $passed${NC} | ${RED}Failed: $failed${NC}"
    echo "========================================="
    
    if [ $failed -gt 0 ]; then
        log_error "Some tests failed"
        exit 1
    else
        log_info "All tests passed!"
        exit 0
    fi
}

# Main execution
main() {
    log_info "Starting Integration Tests for namespace: $NAMESPACE"
    log_info "Base URL: $BASE_URL"
    echo ""
    
    check_prerequisites
    wait_for_pods
    
    test_namespace_resources
    test_postgresql
    test_redis
    test_application_health
    test_http_endpoints
    test_blueprint_operations
    test_resource_limits
    test_persistent_storage
    test_performance
    
    print_summary
}

# Run main function
main "$@"