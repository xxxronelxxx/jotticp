-- Migration 0017: Add missing columns for compile alignment
-- Fixes compile errors in api/cron.rs, services/monitor.rs, and services/jobs.rs

-- cron_jobs: soft-delete support (api/cron.rs uses deleted_at IS NULL)
ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- server_metrics: per-site columns (services/monitor.rs inserts/queries these)
ALTER TABLE server_metrics ADD COLUMN IF NOT EXISTS site_id       UUID REFERENCES sites(id) ON DELETE CASCADE;
ALTER TABLE server_metrics ADD COLUMN IF NOT EXISTS memory_bytes  BIGINT;
ALTER TABLE server_metrics ADD COLUMN IF NOT EXISTS disk_bytes    BIGINT;

-- Index to speed up per-site metric lookups
CREATE INDEX IF NOT EXISTS idx_server_metrics_site
    ON server_metrics(site_id, recorded_at DESC)
    WHERE site_id IS NOT NULL;

-- user_databases: updated_at column (services/jobs.rs uses SET updated_at = NOW())
ALTER TABLE user_databases ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
