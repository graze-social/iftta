-- Initial database schema for ifthisthenat
-- Run this after PostgreSQL is deployed to k3s

-- Blueprints table for storing automation workflows
CREATE TABLE IF NOT EXISTS blueprints (
    id SERIAL PRIMARY KEY,
    uri VARCHAR(255) UNIQUE NOT NULL,
    did VARCHAR(255) NOT NULL,
    content JSONB NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Index for faster lookups
CREATE INDEX IF NOT EXISTS idx_blueprints_did ON blueprints(did);
CREATE INDEX IF NOT EXISTS idx_blueprints_uri ON blueprints(uri);

-- Webhook tasks table for managing webhook deliveries
CREATE TABLE IF NOT EXISTS webhook_tasks (
    id SERIAL PRIMARY KEY,
    url VARCHAR(1024) NOT NULL,
    payload JSONB NOT NULL,
    headers JSONB,
    retry_count INT DEFAULT 0,
    max_retries INT DEFAULT 3,
    status VARCHAR(50) DEFAULT 'pending',
    last_error TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    next_retry_at TIMESTAMP,
    completed_at TIMESTAMP
);

-- Index for task scheduling
CREATE INDEX IF NOT EXISTS idx_webhook_tasks_status ON webhook_tasks(status);
CREATE INDEX IF NOT EXISTS idx_webhook_tasks_next_retry ON webhook_tasks(next_retry_at);

-- Cursor states for tracking stream positions
CREATE TABLE IF NOT EXISTS cursor_states (
    id VARCHAR(255) PRIMARY KEY,
    cursor_value TEXT,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- OAuth sessions table
CREATE TABLE IF NOT EXISTS oauth_sessions (
    id SERIAL PRIMARY KEY,
    did VARCHAR(255) NOT NULL,
    access_token TEXT NOT NULL,
    refresh_token TEXT,
    expires_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Index for OAuth lookups
CREATE INDEX IF NOT EXISTS idx_oauth_sessions_did ON oauth_sessions(did);
CREATE INDEX IF NOT EXISTS idx_oauth_sessions_expires ON oauth_sessions(expires_at);

-- Scheduled tasks for periodic entries
CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id SERIAL PRIMARY KEY,
    blueprint_uri VARCHAR(255) NOT NULL,
    node_id VARCHAR(255) NOT NULL,
    schedule VARCHAR(255) NOT NULL,
    last_run TIMESTAMP,
    next_run TIMESTAMP NOT NULL,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(blueprint_uri, node_id)
);

-- Index for scheduler
CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_next_run ON scheduled_tasks(next_run);
CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_enabled ON scheduled_tasks(enabled);

-- Execution history for debugging
CREATE TABLE IF NOT EXISTS execution_history (
    id SERIAL PRIMARY KEY,
    blueprint_uri VARCHAR(255) NOT NULL,
    trigger_type VARCHAR(50) NOT NULL,
    trigger_data JSONB,
    result JSONB,
    status VARCHAR(50) NOT NULL,
    started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    duration_ms INT
);

-- Index for history queries
CREATE INDEX IF NOT EXISTS idx_execution_history_blueprint ON execution_history(blueprint_uri);
CREATE INDEX IF NOT EXISTS idx_execution_history_started ON execution_history(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_execution_history_status ON execution_history(status);

-- Function to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Create triggers for updated_at
CREATE TRIGGER update_blueprints_updated_at BEFORE UPDATE ON blueprints
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_oauth_sessions_updated_at BEFORE UPDATE ON oauth_sessions
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_scheduled_tasks_updated_at BEFORE UPDATE ON scheduled_tasks
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Grant permissions (adjust as needed)
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO ifthisthenat;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO ifthisthenat;