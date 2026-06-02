-- Migration 0016: Additional schema alignment fixes

-- Fix app_installs table: add deleted_at for soft delete (app_installations view needs it)
ALTER TABLE app_installs ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- Update app_installations view to include deleted_at and created_at with all needed columns
DROP VIEW IF EXISTS app_installations;
CREATE OR REPLACE VIEW app_installations AS
SELECT 
    id,
    site_id,
    manifest_slug AS app_id,
    installed_version AS version,
    status,
    installed_at,
    installed_at AS created_at,
    updated_at,
    deleted_at,
    metadata->>'admin_url' AS admin_url,
    metadata::text AS params
FROM app_installs;

-- Update branding view to include panel_name
DROP VIEW IF EXISTS branding;
CREATE OR REPLACE VIEW branding AS
SELECT
    id,
    reseller_id,
    brand_name AS company_name,
    brand_name,
    brand_name AS panel_name,
    panel_domain,
    logo_path,
    favicon_path,
    primary_color,
    accent_color,
    support_email,
    support_url,
    hide_orbitcp_credit,
    custom_css,
    created_at,
    updated_at
FROM reseller_brands;

-- servers: agent_version column
ALTER TABLE servers ADD COLUMN IF NOT EXISTS agent_version VARCHAR(32);

-- sites: runtime_version (alias for runtime) and memory_bytes
ALTER TABLE sites ADD COLUMN IF NOT EXISTS runtime_version VARCHAR(64);
ALTER TABLE sites ADD COLUMN IF NOT EXISTS memory_bytes BIGINT NOT NULL DEFAULT 0;
UPDATE sites SET runtime_version = runtime WHERE runtime_version IS NULL AND runtime IS NOT NULL;

-- audit_log: resource_id column (alias for target_id)
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS resource_id UUID;
UPDATE audit_log_y2026m01 SET resource_id = target_id WHERE resource_id IS NULL;
UPDATE audit_log_y2026m05 SET resource_id = target_id WHERE resource_id IS NULL;
UPDATE audit_log_y2026m06 SET resource_id = target_id WHERE resource_id IS NULL;

-- user_databases: port alias for db_port
ALTER TABLE user_databases ADD COLUMN IF NOT EXISTS port INT;
UPDATE user_databases SET port = db_port WHERE port IS NULL;

-- server_metrics: ensure site_id is NOT the issue — it doesn't have site_id, but the code queries it via server_id
-- Add missing columns needed by monitor.rs
ALTER TABLE server_metrics ADD COLUMN IF NOT EXISTS load_avg NUMERIC(6,2);
ALTER TABLE server_metrics ADD COLUMN IF NOT EXISTS bandwidth_in_bytes BIGINT;
ALTER TABLE server_metrics ADD COLUMN IF NOT EXISTS bandwidth_out_bytes BIGINT;

-- panel_settings: add panel_name as a row, and add a column for convenience queries
-- panel_name is stored as a row in panel_settings, not a column
-- The code uses panel_settings.panel_name column — add it
ALTER TABLE panel_settings ADD COLUMN IF NOT EXISTS panel_name TEXT;

-- Fix site_databases view to include status
DROP VIEW IF EXISTS site_databases;
CREATE OR REPLACE VIEW site_databases AS
SELECT 
    id,
    site_id,
    server_id,
    owner_id,
    db_type::text AS db_type,
    db_name,
    db_user,
    db_host,
    db_port,
    size_mb,
    created_at,
    updated_at,
    deleted_at,
    'active'::text AS status
FROM databases;

-- user_database_stats: rebuilt with more columns
DROP VIEW IF EXISTS user_database_stats;
CREATE OR REPLACE VIEW user_database_stats AS
SELECT 
    ud.id,
    ud.site_id,
    ud.db_type,
    ud.db_name,
    ud.db_user,
    ud.status,
    ud.db_host,
    ud.db_port,
    ud.port,
    ud.connection_string,
    ud.created_at,
    ud.deleted_at
FROM user_databases ud
WHERE ud.deleted_at IS NULL;
