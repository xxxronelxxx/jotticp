-- Migration 0013: Add missing columns and tables
-- This migration is idempotent (uses IF NOT EXISTS / ADD COLUMN IF NOT EXISTS).
-- It MUST NOT be applied to databases that already have these objects from
-- a schema.sql seed — run `sqlx migrate run` which skips already-applied
-- migrations via the _sqlx_migrations table.
--
-- Fixes: BUG-012, BUG-014, BUG-015, BUG-016, BUG-017

-- ── BUG-012: email_accounts missing columns ───────────────────────────────────
-- NOTE: email_accounts.used_mb already exists in 0001_initial.sql.
-- ADD COLUMN IF NOT EXISTS is idempotent and will no-op if the column is present.
ALTER TABLE email_accounts
    ADD COLUMN IF NOT EXISTS used_mb        INTEGER     NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS spam_threshold NUMERIC(5,2) NOT NULL DEFAULT 5.0;

-- BUG-012: email_autoresponders missing deleted_at
ALTER TABLE email_autoresponders
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- BUG-012: email_forwarders missing deleted_at
ALTER TABLE email_forwarders
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- ── BUG-014: backup_settings table ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS backup_settings (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    daily_time      TIME         NOT NULL DEFAULT '03:00',
    retention_days  INTEGER      NOT NULL DEFAULT 7,
    retention_weeks INTEGER      NOT NULL DEFAULT 4,
    destination     TEXT         NOT NULL DEFAULT 'local',
    s3_bucket       TEXT,
    s3_region       TEXT,
    s3_access_key   TEXT,
    s3_secret_key   TEXT,
    sftp_host       TEXT,
    sftp_user       TEXT,
    sftp_path       TEXT,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (site_id)
);

-- BUG-014: backup_jobs table (referenced by backups.rs alongside backup_settings)
CREATE TABLE IF NOT EXISTS backup_jobs (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    backup_type     TEXT         NOT NULL DEFAULT 'full',
    status          TEXT         NOT NULL DEFAULT 'pending',
    size_bytes      BIGINT,
    destination     TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    manifest_path   TEXT,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_backup_jobs_site
    ON backup_jobs(site_id, created_at DESC);

-- ── BUG-015: site_settings table ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS site_settings (
    id            UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id       UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    setting_key   TEXT         NOT NULL,
    setting_value TEXT         NOT NULL,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (site_id, setting_key)
);

CREATE INDEX IF NOT EXISTS idx_site_settings_site
    ON site_settings(site_id);

-- ── BUG-016: site_php_extensions table ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS site_php_extensions (
    id             UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id        UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    extension_name TEXT         NOT NULL,
    enabled        BOOLEAN      NOT NULL DEFAULT true,
    UNIQUE (site_id, extension_name)
);

CREATE INDEX IF NOT EXISTS idx_site_php_extensions_site
    ON site_php_extensions(site_id);

-- BUG-016: site_php_settings table
CREATE TABLE IF NOT EXISTS site_php_settings (
    id            UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id       UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    setting_key   TEXT         NOT NULL,
    setting_value TEXT         NOT NULL,
    UNIQUE (site_id, setting_key)
);

CREATE INDEX IF NOT EXISTS idx_site_php_settings_site
    ON site_php_settings(site_id);

-- ── BUG-017: dns_zones owner_id column ───────────────────────────────────────
-- owner_id already exists as NOT NULL in 0001_initial.sql.
-- ADD COLUMN IF NOT EXISTS is a no-op when the column is present, so this
-- is safe to apply against both old and new databases.
ALTER TABLE dns_zones
    ADD COLUMN IF NOT EXISTS owner_id UUID REFERENCES users(id);
