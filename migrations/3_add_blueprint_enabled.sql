-- Add enabled column to blueprints table
-- Default to true for existing blueprints
ALTER TABLE public.blueprints ADD COLUMN enabled boolean NOT NULL DEFAULT true;

-- Add index for efficient filtering of enabled blueprints
CREATE INDEX idx_blueprints_enabled ON public.blueprints USING btree (enabled);