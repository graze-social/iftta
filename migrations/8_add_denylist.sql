-- Create denylist table for storing blocked items
CREATE TABLE IF NOT EXISTS denylist (
    -- The item being denied (e.g., DID, handle, URL, or any string identifier)
    item TEXT PRIMARY KEY,

    -- When this item was added to the denylist
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Optional note explaining why this item was denied
    note TEXT
);

-- Create index on created_at for efficient time-based queries
CREATE INDEX IF NOT EXISTS idx_denylist_created_at ON denylist(created_at DESC);

-- Add comment to table
COMMENT ON TABLE denylist IS 'Storage for denied/blocked items with optional notes';
COMMENT ON COLUMN denylist.item IS 'The denied item (primary key)';
COMMENT ON COLUMN denylist.created_at IS 'Timestamp when the item was added to the denylist';
COMMENT ON COLUMN denylist.note IS 'Optional explanation for why this item was denied';