-- Migration: Add prototypes table for blueprint templates
-- 
-- Prototypes are blueprint templates with placeholders that can be instantiated
-- into concrete blueprints by providing values for the placeholders.

-- Create the prototypes table
CREATE TABLE IF NOT EXISTS prototypes (
    -- Unique AT-URI for the prototype
    aturi TEXT PRIMARY KEY,
    
    -- DID of the prototype owner
    did TEXT NOT NULL,
    
    -- Human-readable name for the prototype
    name TEXT NOT NULL,
    
    -- Description of what this prototype does
    description TEXT NOT NULL,
    
    -- The node templates with placeholders (stored as JSONB)
    nodes JSONB NOT NULL,
    
    -- Placeholder definitions (stored as JSONB array)
    placeholders JSONB NOT NULL DEFAULT '[]'::jsonb,
    
    -- Creation timestamp
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Last update timestamp
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for querying prototypes by owner
CREATE INDEX idx_prototypes_did ON prototypes(did);

-- Index for sorting by creation date
CREATE INDEX idx_prototypes_created_at ON prototypes(created_at DESC);

-- Index for full-text search on name and description
CREATE INDEX idx_prototypes_search ON prototypes USING GIN(
    to_tsvector('english', name || ' ' || description)
);

-- Function to update the updated_at timestamp
CREATE OR REPLACE FUNCTION update_prototype_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to automatically update updated_at on row updates
CREATE TRIGGER trg_prototypes_updated_at
    BEFORE UPDATE ON prototypes
    FOR EACH ROW
    EXECUTE FUNCTION update_prototype_updated_at();

-- Table for tracking prototype instantiations (optional - for analytics)
CREATE TABLE IF NOT EXISTS prototype_instantiations (
    id SERIAL PRIMARY KEY,
    
    -- Reference to the prototype
    prototype_aturi TEXT NOT NULL REFERENCES prototypes(aturi) ON DELETE CASCADE,
    
    -- The blueprint that was created
    blueprint_aturi TEXT NOT NULL,
    
    -- Who instantiated it
    did TEXT NOT NULL,
    
    -- The values used for instantiation (stored for history)
    placeholder_values JSONB NOT NULL,
    
    -- When it was instantiated
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for querying instantiations by prototype
CREATE INDEX idx_instantiations_prototype ON prototype_instantiations(prototype_aturi);

-- Index for querying instantiations by user
CREATE INDEX idx_instantiations_did ON prototype_instantiations(did);

-- Index for querying instantiations by time
CREATE INDEX idx_instantiations_created_at ON prototype_instantiations(created_at DESC);

-- Comments for documentation
COMMENT ON TABLE prototypes IS 'Blueprint templates with placeholders for creating reusable automation patterns';
COMMENT ON COLUMN prototypes.aturi IS 'Unique AT-URI identifier for the prototype';
COMMENT ON COLUMN prototypes.did IS 'DID of the user who owns this prototype';
COMMENT ON COLUMN prototypes.name IS 'Human-readable name for the prototype';
COMMENT ON COLUMN prototypes.description IS 'Description of what this prototype does and how to use it';
COMMENT ON COLUMN prototypes.nodes IS 'JSON structure of nodes with $[PLACEHOLDER] markers';
COMMENT ON COLUMN prototypes.placeholders IS 'Array of placeholder definitions including type, validation, and defaults';

COMMENT ON TABLE prototype_instantiations IS 'History of prototype instantiations for analytics and debugging';
COMMENT ON COLUMN prototype_instantiations.prototype_aturi IS 'The prototype that was instantiated';
COMMENT ON COLUMN prototype_instantiations.blueprint_aturi IS 'The resulting blueprint AT-URI';
COMMENT ON COLUMN prototype_instantiations.placeholder_values IS 'The values used for placeholders during instantiation';