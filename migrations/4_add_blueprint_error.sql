-- Add error column to blueprints table
-- Default to NULL for existing blueprints
ALTER TABLE public.blueprints ADD COLUMN error text DEFAULT NULL;

-- Add index for efficient filtering of errored blueprints
CREATE INDEX idx_blueprints_error ON public.blueprints USING btree (error) WHERE error IS NOT NULL;