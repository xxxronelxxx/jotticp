-- Migration 0005: Database and cron supplemental tables
-- `databases` and `cron_jobs` are already created in 0001_initial.sql.
-- This migration adds:
--   • user_database_credentials — per-site DB credential vault
--   • database_backups          — DB-specific backup manifest rows
--   • cron_run_log              — per-execution cron history (separate from last_run_* cols)

-- ── Database credential vault ─────────────────────────────────────────────────
-- The databases table stores the logical DB record.
-- This table stores the encrypted credentials returned to the user once
-- (on creation) and stored for panel-managed operations (mysqldump, etc.).
CREATE TABLE IF NOT EXISTS database_credentials (
    id          UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    database_id UUID    NOT NULL UNIQUE REFERENCES databases(id) ON DELETE CASCADE,
    db_password TEXT    NOT NULL,   -- argon2-encrypted or envelope-encrypted
    connection_string TEXT,         -- pre-formatted connection URL (password redacted)
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rotated_at  TIMESTAMPTZ
);

-- ── Valkey/Redis per-site configuration ───────────────────────────────────────
-- Per-site Valkey instances (Step 4.5). Valkey uses Unix sockets,
-- not TCP ports by default. The databases table uses db_type but
-- Valkey needs a socket path rather than a TCP connection string.
CREATE TABLE IF NOT EXISTS valkey_instances (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID    NOT NULL UNIQUE REFERENCES sites(id) ON DELETE CASCADE,
    socket_path     TEXT    NOT NULL,    -- /var/run/jotticp/valkey-SITEID.sock
    password_hash   TEXT    NOT NULL,    -- requirepass value
    maxmemory_mb    INT     NOT NULL DEFAULT 32,
    is_active       BOOLEAN NOT NULL DEFAULT FALSE,
    started_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── Cron job execution log ────────────────────────────────────────────────────
-- cron_jobs.last_run_at tracks only the last run.
-- This table retains the last N executions (pruned by orbit-cron after 90 days).
CREATE TABLE IF NOT EXISTS cron_run_log (
    id          BIGSERIAL    PRIMARY KEY,
    job_id      UUID         NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    started_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    exit_code   INT,
    output      TEXT,        -- truncated to 64KB
    error       TEXT         -- stderr, truncated to 16KB
);

CREATE INDEX IF NOT EXISTS idx_cron_run_log_job
    ON cron_run_log(job_id, started_at DESC);

-- Prune rows older than 90 days (called by orbit-cron daily sweep)
-- Implemented as a function so orbit-cron can CALL it without a full table scan.
CREATE OR REPLACE FUNCTION prune_cron_run_log() RETURNS void AS $$
BEGIN
    DELETE FROM cron_run_log WHERE started_at < NOW() - INTERVAL '90 days';
END;
$$ LANGUAGE plpgsql;

-- ── SurrealDB per-site configuration ─────────────────────────────────────────
-- Pro plan (Step 4.6). SurrealDB runs on a unique loopback port per site.
CREATE TABLE IF NOT EXISTS surrealdb_instances (
    id          UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id     UUID    NOT NULL UNIQUE REFERENCES sites(id) ON DELETE CASCADE,
    port        INT     NOT NULL UNIQUE CHECK (port BETWEEN 49152 AND 65535),
    data_dir    TEXT    NOT NULL,    -- /var/jotticp/surrealdb/SITEID/
    root_user   TEXT    NOT NULL DEFAULT 'root',
    root_pass   TEXT    NOT NULL,    -- encrypted at rest
    is_active   BOOLEAN NOT NULL DEFAULT FALSE,
    idle_since  TIMESTAMPTZ,        -- auto-stop after 15 min idle
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
