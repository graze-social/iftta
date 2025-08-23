-- Add session_type column to sessions table to distinguish between OAuth and app-password sessions

-- Add the new column with a default value of 'oauth'
ALTER TABLE public.sessions 
ADD COLUMN session_type text NOT NULL DEFAULT 'oauth';

-- Add a check constraint to ensure only valid session types
ALTER TABLE public.sessions 
ADD CONSTRAINT check_session_type 
CHECK (session_type IN ('oauth', 'app_password'));

-- Create an index for querying by session type if needed
CREATE INDEX idx_sessions_type ON public.sessions USING btree (session_type);

-- Add comment to document the column
COMMENT ON COLUMN public.sessions.session_type IS 'Type of authentication used for this session: oauth or app_password';