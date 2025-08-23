// k6 Load Testing Script for ifthisthenat
// Run with: k6 run k6-load-test.js

import http from 'k6/http';
import { check, group, sleep } from 'k6';
import { Rate } from 'k6/metrics';

// Custom metrics
const errorRate = new Rate('errors');
const apiErrorRate = new Rate('api_errors');

// Test configuration
export let options = {
    stages: [
        { duration: '30s', target: 5 },   // Ramp up to 5 users
        { duration: '1m', target: 10 },   // Stay at 10 users
        { duration: '2m', target: 20 },   // Ramp up to 20 users
        { duration: '2m', target: 20 },   // Stay at 20 users
        { duration: '1m', target: 10 },   // Ramp down to 10 users
        { duration: '30s', target: 0 },   // Ramp down to 0 users
    ],
    thresholds: {
        http_req_duration: ['p(95)<500'],  // 95% of requests must complete below 500ms
        http_req_failed: ['rate<0.1'],     // Error rate must be below 10%
        errors: ['rate<0.1'],               // Custom error rate below 10%
    },
};

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';

// Test data
const testBlueprint = {
    uri: `at://did:plc:test/tools.graze.ifthisthenat.blueprint/loadtest-${Math.random()}`,
    content: {
        name: 'Load Test Blueprint',
        order: 1,
        nodes: [
            {
                id: 'entry',
                type: 'webhook_entry',
                payload: {
                    path: '/test'
                }
            },
            {
                id: 'condition',
                type: 'condition',
                payload: {
                    expression: {
                        '==': [
                            { val: ['data', 'action'] },
                            'test'
                        ]
                    }
                }
            },
            {
                id: 'debug',
                type: 'debug_action',
                payload: {
                    message: 'Load test action'
                }
            }
        ]
    }
};

const webhookPayload = {
    action: 'test',
    timestamp: new Date().toISOString(),
    data: {
        test: true,
        value: Math.random()
    }
};

// Helper function to make authenticated requests (if needed)
function makeRequest(url, params = {}) {
    // Add any required headers here
    params.headers = params.headers || {};
    params.headers['Content-Type'] = 'application/json';
    
    // Add authentication if available
    if (__ENV.AUTH_TOKEN) {
        params.headers['Authorization'] = `Bearer ${__ENV.AUTH_TOKEN}`;
    }
    
    return http.request(params.method || 'GET', url, params.body ? JSON.stringify(params.body) : null, {
        headers: params.headers,
        timeout: '30s'
    });
}

// Test scenarios
export default function() {
    group('Health Check', function() {
        const response = http.get(`${BASE_URL}/`);
        const success = check(response, {
            'health check status is 200 or 404': (r) => r.status === 200 || r.status === 404,
            'health check response time < 200ms': (r) => r.timings.duration < 200,
        });
        errorRate.add(!success);
    });
    
    sleep(1);
    
    group('XRPC API Tests', function() {
        // Test get blueprints endpoint
        const getResponse = makeRequest(`${BASE_URL}/xrpc/tools.graze.ifthisthenat.getBlueprints`);
        const getSuccess = check(getResponse, {
            'get blueprints status is 200, 401, or 400': (r) => 
                r.status === 200 || r.status === 401 || r.status === 400,
            'get blueprints response time < 500ms': (r) => r.timings.duration < 500,
        });
        apiErrorRate.add(!getSuccess);
        
        sleep(0.5);
        
        // Test create blueprint endpoint
        const createResponse = makeRequest(`${BASE_URL}/xrpc/tools.graze.ifthisthenat.updateBlueprint`, {
            method: 'POST',
            body: testBlueprint
        });
        const createSuccess = check(createResponse, {
            'create blueprint status is acceptable': (r) => 
                r.status === 200 || r.status === 201 || r.status === 401 || r.status === 400,
            'create blueprint response time < 1000ms': (r) => r.timings.duration < 1000,
        });
        apiErrorRate.add(!createSuccess);
    });
    
    sleep(1);
    
    group('Webhook Tests', function() {
        const webhookResponse = makeRequest(`${BASE_URL}/webhook/test`, {
            method: 'POST',
            body: webhookPayload
        });
        const webhookSuccess = check(webhookResponse, {
            'webhook status is acceptable': (r) => 
                r.status === 200 || r.status === 202 || r.status === 401 || r.status === 404,
            'webhook response time < 300ms': (r) => r.timings.duration < 300,
        });
        errorRate.add(!webhookSuccess);
    });
    
    sleep(2);
    
    // Occasionally test OAuth callback (less frequently)
    if (Math.random() < 0.1) {
        group('OAuth Callback Test', function() {
            const oauthResponse = http.get(`${BASE_URL}/oauth/callback?code=test&state=test`);
            check(oauthResponse, {
                'oauth callback responds': (r) => r.status > 0,
            });
        });
    }
}

// Lifecycle hooks
export function setup() {
    console.log('Starting load test...');
    console.log(`Base URL: ${BASE_URL}`);
    
    // Test connectivity
    const response = http.get(`${BASE_URL}/`);
    if (response.status === 0) {
        throw new Error(`Cannot connect to ${BASE_URL}`);
    }
    
    return { startTime: new Date() };
}

export function teardown(data) {
    console.log('Load test completed');
    const duration = new Date() - data.startTime;
    console.log(`Total duration: ${duration / 1000}s`);
}

// Custom summary (optional)
export function handleSummary(data) {
    return {
        'stdout': textSummary(data, { indent: ' ', enableColors: true }),
        'summary.json': JSON.stringify(data, null, 2),
        'summary.html': htmlReport(data),
    };
}

// Helper function for text summary
function textSummary(data, options) {
    // This would normally use k6's textSummary function
    // Simplified version for demonstration
    let summary = '\n=== Load Test Summary ===\n\n';
    
    if (data.metrics) {
        summary += 'Metrics:\n';
        for (const [key, value] of Object.entries(data.metrics)) {
            if (value.values) {
                summary += `  ${key}:\n`;
                if (value.values.avg !== undefined) {
                    summary += `    avg: ${value.values.avg.toFixed(2)}ms\n`;
                }
                if (value.values.p95 !== undefined) {
                    summary += `    p95: ${value.values.p(95).toFixed(2)}ms\n`;
                }
            }
        }
    }
    
    return summary;
}

// Helper function for HTML report
function htmlReport(data) {
    return `
<!DOCTYPE html>
<html>
<head>
    <title>Load Test Report</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; }
        h1 { color: #333; }
        .metrics { margin: 20px 0; }
        .metric { padding: 10px; margin: 5px 0; background: #f5f5f5; }
        .pass { color: green; }
        .fail { color: red; }
    </style>
</head>
<body>
    <h1>ifthisthenat Load Test Report</h1>
    <div class="metrics">
        <h2>Test Results</h2>
        <pre>${JSON.stringify(data, null, 2)}</pre>
    </div>
</body>
</html>
    `;
}