-- Migration: Add scheduler state table for periodic blueprint execution
-- This table tracks when blueprints with periodic_entry nodes were last run
-- and when they should next be executed based on their cron schedules.

-- Create the scheduler_state table
CREATE TABLE IF NOT EXISTS scheduler_state (
    -- Primary key: the blueprint ID (aturi)
    blueprint_id TEXT PRIMARY KEY,
    
    -- When the blueprint was last successfully executed
    -- NULL if it has never been run
    last_run TIMESTAMPTZ,
    
    -- When the blueprint should next be executed
    -- Required field as we always need to know when to run next
    next_run TIMESTAMPTZ NOT NULL,
    
    -- The cron expression that defines the schedule
    -- Examples: "0 * * * *" (every hour), "0 0 9 * * *" (daily at 9 AM)
    cron_expression TEXT NOT NULL,
    
    -- Whether this scheduled blueprint is enabled
    -- Can be used to temporarily disable scheduled execution
    enabled BOOLEAN NOT NULL DEFAULT true,
    
    -- Audit timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create index for efficient queries to find blueprints ready to run
-- Only indexes enabled blueprints since disabled ones won't be queried
CREATE INDEX IF NOT EXISTS idx_scheduler_state_next_run 
    ON scheduler_state (next_run) 
    WHERE enabled = true;

-- Create index for quick lookups by blueprint_id
-- (Primary key already creates an index, but being explicit for clarity)
CREATE INDEX IF NOT EXISTS idx_scheduler_state_blueprint_id 
    ON scheduler_state (blueprint_id);

-- Add comment to table for documentation
COMMENT ON TABLE scheduler_state IS 'Tracks execution state for blueprints with periodic_entry nodes that run on cron schedules';

-- Add column comments for documentation
COMMENT ON COLUMN scheduler_state.blueprint_id IS 'Foreign key reference to blueprint.aturi';
COMMENT ON COLUMN scheduler_state.last_run IS 'Timestamp of the last successful execution, NULL if never run';
COMMENT ON COLUMN scheduler_state.next_run IS 'Timestamp when the blueprint should next be executed based on cron schedule';
COMMENT ON COLUMN scheduler_state.cron_expression IS 'Cron expression defining the execution schedule (supports 5 or 6 field format)';
COMMENT ON COLUMN scheduler_state.enabled IS 'Whether this scheduled blueprint is currently active';
COMMENT ON COLUMN scheduler_state.created_at IS 'When this scheduler state entry was first created';
COMMENT ON COLUMN scheduler_state.updated_at IS 'When this scheduler state entry was last modified';

-- Create trigger to automatically update the updated_at timestamp
CREATE OR REPLACE FUNCTION update_scheduler_state_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_scheduler_state_updated_at
    BEFORE UPDATE ON scheduler_state
    FOR EACH ROW
    EXECUTE FUNCTION update_scheduler_state_updated_at();