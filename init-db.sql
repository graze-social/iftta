-- Initialize databases for iftta development
-- This script runs when the PostgreSQL container is first created

-- Create test database for running tests
CREATE DATABASE iftta_test;

-- Grant permissions to postgres user (already has superuser, but being explicit)
GRANT ALL PRIVILEGES ON DATABASE iftta TO postgres;
GRANT ALL PRIVILEGES ON DATABASE iftta_test TO postgres;