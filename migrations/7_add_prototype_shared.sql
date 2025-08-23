-- Add shared field to prototypes table
-- This field indicates whether a prototype is publicly available to all users
ALTER TABLE prototypes 
ADD COLUMN shared BOOLEAN NOT NULL DEFAULT false;

-- Create index for efficient queries of shared prototypes
CREATE INDEX idx_prototypes_shared ON prototypes(shared) WHERE shared = true;

-- Add comment to document the column
COMMENT ON COLUMN prototypes.shared IS 'Indicates if this prototype is publicly available to all users';