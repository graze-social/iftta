-- Initial schema for ifthisthenat application
-- This migration combines all previous migrations into a single initial schema

-- Identities table for caching DID documents
CREATE TABLE public.identities (
    did text NOT NULL,
    handle text,
    document jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (did)
);

CREATE INDEX idx_identities_handle ON public.identities USING btree (handle);

-- Blueprints table for storing blueprint definitions
CREATE TABLE public.blueprints (
    aturi text NOT NULL,
    did text NOT NULL,
    node_order jsonb DEFAULT '[]'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (aturi)
);

CREATE INDEX idx_blueprints_did ON public.blueprints USING btree (did);

-- Nodes table for storing blueprint nodes
CREATE TABLE public.nodes (
    aturi text NOT NULL,
    blueprint text NOT NULL REFERENCES public.blueprints(aturi) ON DELETE CASCADE,
    type text NOT NULL,
    payload jsonb NOT NULL,
    configuration jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (aturi)
);

CREATE INDEX idx_nodes_blueprint ON public.nodes USING btree (blueprint);

-- ATProto OAuth requests table for OAuth flow state
CREATE TABLE public.atpoauth_requests (
    oauth_state text NOT NULL,
    issuer text NOT NULL,
    authorization_server text NOT NULL,
    did text NOT NULL,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    secret_jwk_id text NOT NULL,
    destination text,
    dpop_jwk text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    PRIMARY KEY (oauth_state)
);

CREATE INDEX idx_atpoauth_requests_did ON public.atpoauth_requests USING btree (did);
CREATE INDEX idx_atpoauth_requests_expires_at ON public.atpoauth_requests USING btree (expires_at);

-- ATProto OAuth sessions table
CREATE TABLE public.atpoauth_sessions (
    session_chain text NOT NULL,
    access_token text NOT NULL,
    did text NOT NULL,
    issuer text NOT NULL,
    refresh_token text NOT NULL,
    secret_jwk_id text NOT NULL,
    dpop_jwk text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    access_token_expires_at timestamp with time zone NOT NULL,
    PRIMARY KEY (session_chain)
);

CREATE INDEX idx_atpoauth_sessions_access_token_expires_at ON public.atpoauth_sessions USING btree (access_token_expires_at);

-- Sessions table for storing authenticated user sessions
CREATE TABLE public.sessions (
    session_id text NOT NULL,
    did text NOT NULL,
    access_token text NOT NULL,
    refresh_token text,
    access_token_expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (session_id)
);

CREATE INDEX idx_sessions_did ON public.sessions USING btree (did);
CREATE INDEX idx_sessions_expires_at ON public.sessions USING btree (access_token_expires_at);

-- OAuth requests table for storing OAuth flow state
CREATE TABLE public.oauth_requests (
    oauth_state text NOT NULL,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    PRIMARY KEY (oauth_state)
);

CREATE INDEX idx_oauth_requests_expires_at ON public.oauth_requests USING btree (expires_at);