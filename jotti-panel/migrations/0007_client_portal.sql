-- Migration 0007: Client portal, reseller packages, and migration jobs
-- Steps 4.13, 4.14, 4.15, 4.18, 4.19

-- ── Migration jobs table (Step 4.15) ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS migration_jobs (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_site_id UUID        REFERENCES sites(id) ON DELETE SET NULL,
    status         VARCHAR(30) NOT NULL DEFAULT 'queued',
        -- queued | running | completed | completed_with_errors | failed
    progress       INTEGER     NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    archive_path   TEXT,
    report         JSONB,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_migration_jobs_user_id    ON migration_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_migration_jobs_status     ON migration_jobs(status);
CREATE INDEX IF NOT EXISTS idx_migration_jobs_created_at ON migration_jobs(created_at DESC);

-- ── Reseller packages table (Steps 4.14, 4.19) ───────────────────────────────
CREATE TABLE IF NOT EXISTS reseller_packages (
    id                   UUID           PRIMARY KEY DEFAULT gen_random_uuid(),
    reseller_id          UUID           REFERENCES users(id) ON DELETE CASCADE,
        -- NULL means a global/admin-owned package available to all resellers
    name                 VARCHAR(100)   NOT NULL,
    max_sites            INTEGER        NOT NULL DEFAULT 10    CHECK (max_sites >= 0),
    max_disk_gb          INTEGER        NOT NULL DEFAULT 50    CHECK (max_disk_gb >= 0),
    max_email_accounts   INTEGER        NOT NULL DEFAULT 50    CHECK (max_email_accounts >= 0),
    max_databases        INTEGER        NOT NULL DEFAULT 10    CHECK (max_databases >= 0),
    price_monthly        DECIMAL(10,2)  NOT NULL DEFAULT 0.00  CHECK (price_monthly >= 0),
    -- Feature flags (Step 4.19): JSONB for forward-compatible flag addition
    feature_flags        JSONB          NOT NULL DEFAULT '{
        "dns_editing":        true,
        "php_version_switch": true,
        "app_installer":      true,
        "ssh_access":         false,
        "staging":            false,
        "db_manager":         true,
        "file_manager":       true
    }',
    created_at           TIMESTAMPTZ    NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_reseller_packages_reseller_id ON reseller_packages(reseller_id);

-- ── User package assignments (Step 4.13) ────────────────────────────────────
-- One package per user (PRIMARY KEY on user_id enforces this).
-- ON CONFLICT … DO UPDATE in the assign_package handler handles reassignment.
CREATE TABLE IF NOT EXISTS user_packages (
    user_id     UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    package_id  UUID        NOT NULL    REFERENCES reseller_packages(id) ON DELETE RESTRICT,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_user_packages_package_id ON user_packages(package_id);

-- ── Ensure audit_log has an ip_address column (Step 4.18) ───────────────────
-- (Already present in most installations; added here as idempotent safety net)
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS ip_address TEXT;
