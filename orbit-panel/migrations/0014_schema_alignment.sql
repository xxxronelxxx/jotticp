-- 0014_schema_alignment.sql
-- Aligns schema with Rust code expectations: adds missing columns, tables, views, and enum variants.

-- ============================================================
-- USERS TABLE
-- ============================================================
ALTER TABLE users ADD COLUMN IF NOT EXISTS full_name TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS status VARCHAR(32) NOT NULL DEFAULT 'active';

-- ============================================================
-- SERVERS TABLE
-- The Rust code uses ip_address, address, and agent_address
-- ============================================================
ALTER TABLE servers ADD COLUMN IF NOT EXISTS ip_address INET;
UPDATE servers SET ip_address = ip WHERE ip_address IS NULL;
ALTER TABLE servers ADD COLUMN IF NOT EXISTS address TEXT;
UPDATE servers SET address = hostname WHERE address IS NULL;
ALTER TABLE servers ADD COLUMN IF NOT EXISTS agent_address TEXT;
UPDATE servers SET agent_address = hostname || ':' || agent_port WHERE agent_address IS NULL;

-- ============================================================
-- CRON_JOBS TABLE
-- Code expects individual cron fields; schema has single schedule column
-- ============================================================
ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS minute  VARCHAR(10) NOT NULL DEFAULT '*';
ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS hour    VARCHAR(10) NOT NULL DEFAULT '*';
ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS day     VARCHAR(10) NOT NULL DEFAULT '*';
ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS month   VARCHAR(10) NOT NULL DEFAULT '*';
ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS weekday VARCHAR(10) NOT NULL DEFAULT '*';

-- ============================================================
-- DNS_ZONES TABLE
-- Code uses domain (no trailing dot), deleted_at, ttl, and status
-- ============================================================
ALTER TABLE dns_zones ADD COLUMN IF NOT EXISTS domain     TEXT;
UPDATE dns_zones SET domain = rtrim(zone, '.') WHERE domain IS NULL;
ALTER TABLE dns_zones ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE dns_zones ADD COLUMN IF NOT EXISTS ttl        INT NOT NULL DEFAULT 3600;
ALTER TABLE dns_zones ADD COLUMN IF NOT EXISTS status     VARCHAR(32) NOT NULL DEFAULT 'active';

-- ============================================================
-- DNS_RECORDS TABLE
-- Code uses record_type as alias for type, and deleted_at for soft-delete
-- ============================================================
ALTER TABLE dns_records ADD COLUMN IF NOT EXISTS record_type TEXT;
UPDATE dns_records SET record_type = type WHERE record_type IS NULL;
ALTER TABLE dns_records ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- ============================================================
-- AUDIT_LOG TABLE (partitioned)
-- PostgreSQL 17: ADD COLUMN on parent propagates to partitions
-- Code uses resource_type and detail
-- ============================================================
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS resource_type VARCHAR(64);
UPDATE audit_log SET resource_type = target_type WHERE resource_type IS NULL;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS detail JSONB;
UPDATE audit_log SET detail = details WHERE detail IS NULL;

-- ============================================================
-- EMAIL_ACCOUNTS TABLE
-- ============================================================
ALTER TABLE email_accounts ADD COLUMN IF NOT EXISTS site_id UUID REFERENCES sites(id) ON DELETE SET NULL;
ALTER TABLE email_accounts ADD COLUMN IF NOT EXISTS status  VARCHAR(16) NOT NULL DEFAULT 'active';
-- address is a computed alias for local_part@domain (populated by trigger or view)
ALTER TABLE email_accounts ADD COLUMN IF NOT EXISTS address TEXT;

-- ============================================================
-- EMAIL_FORWARDERS TABLE
-- ============================================================
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS site_id     UUID REFERENCES sites(id) ON DELETE SET NULL;
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS start_at    TIMESTAMPTZ;
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS end_at      TIMESTAMPTZ;
-- source/destination are aliases for from_address/to_address
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS source      TEXT;
UPDATE email_forwarders SET source = from_address WHERE source IS NULL;
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS destination TEXT;
UPDATE email_forwarders SET destination = to_address WHERE destination IS NULL;

-- ============================================================
-- EMAIL_AUTORESPONDERS TABLE
-- ============================================================
ALTER TABLE email_autoresponders ADD COLUMN IF NOT EXISTS start_at TIMESTAMPTZ;
ALTER TABLE email_autoresponders ADD COLUMN IF NOT EXISTS end_at   TIMESTAMPTZ;

-- ============================================================
-- SSL_CERTS TABLE
-- ============================================================
ALTER TABLE ssl_certs ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ;

-- ============================================================
-- SITES TABLE
-- ============================================================
ALTER TABLE sites ADD COLUMN IF NOT EXISTS runtime      VARCHAR(32);
ALTER TABLE sites ADD COLUMN IF NOT EXISTS service_port INT;

-- ============================================================
-- USER_DATABASES TABLE
-- ============================================================
ALTER TABLE user_databases ADD COLUMN IF NOT EXISTS connection_string TEXT;
ALTER TABLE user_databases ADD COLUMN IF NOT EXISTS db_host           INET;
ALTER TABLE user_databases ADD COLUMN IF NOT EXISTS db_port           INT NOT NULL DEFAULT 3306;
ALTER TABLE user_databases ADD COLUMN IF NOT EXISTS db_password       TEXT;

-- ============================================================
-- RESELLER_PACKAGES TABLE
-- Migration 0006 created reseller_packages with individual feat_* columns.
-- Migration 0007 attempted to CREATE TABLE IF NOT EXISTS with price_monthly
-- and feature_flags — but since 0006 ran first, 0007 was a no-op.
-- Add the missing columns here.
-- ============================================================
ALTER TABLE reseller_packages ADD COLUMN IF NOT EXISTS price_monthly  DECIMAL(10,2) NOT NULL DEFAULT 0.00;
ALTER TABLE reseller_packages ADD COLUMN IF NOT EXISTS feature_flags  JSONB         NOT NULL DEFAULT '{
    "dns_editing":        true,
    "php_version_switch": true,
    "app_installer":      true,
    "ssh_access":         false,
    "staging":            false,
    "db_manager":         true,
    "file_manager":       true
}';

-- ============================================================
-- MISSING TABLES
-- ============================================================

CREATE TABLE IF NOT EXISTS panel_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS server_metrics (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id     UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    recorded_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cpu_pct       NUMERIC(5,2),
    ram_used_mb   BIGINT,
    disk_used_gb  BIGINT,
    load_1min     NUMERIC(6,2),
    net_in_bytes  BIGINT,
    net_out_bytes BIGINT
);

CREATE INDEX IF NOT EXISTS idx_server_metrics_server
    ON server_metrics(server_id, recorded_at DESC);

-- ============================================================
-- VIEWS (compatibility aliases)
-- ============================================================

-- app_installations is an alias for app_installs used in some code paths
CREATE OR REPLACE VIEW app_installations AS
SELECT
    id,
    site_id,
    manifest_slug AS app_id,
    installed_version AS version,
    status,
    installed_at,
    updated_at,
    metadata->>'admin_url' AS admin_url,
    metadata->>'params'    AS params
FROM app_installs;

-- branding is an alias for reseller_brands
CREATE OR REPLACE VIEW branding AS
SELECT
    id,
    reseller_id,
    brand_name AS company_name,
    brand_name,
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

-- site_databases is an alias for databases table
CREATE OR REPLACE VIEW site_databases AS SELECT * FROM databases;

-- user_database_stats view
CREATE OR REPLACE VIEW user_database_stats AS
SELECT
    ud.id,
    ud.site_id,
    ud.db_type,
    ud.db_name,
    ud.status,
    ud.created_at
FROM user_databases ud
WHERE ud.deleted_at IS NULL;

-- ============================================================
-- ENUM VARIANTS
-- PostgreSQL requires each ADD VALUE in its own ALTER statement
-- ============================================================
ALTER TYPE site_status    ADD VALUE IF NOT EXISTS 'deleted';
ALTER TYPE ssl_cert_status ADD VALUE IF NOT EXISTS 'error';
