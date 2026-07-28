-- Migration 0006: Backup supplemental tables + branding + blacklist alerts
-- backup_schedules and backups are already in 0001_initial.sql.
-- reseller_brands is also already in 0001_initial.sql.
-- This migration adds:
--   • backup_restore_jobs   — tracks file/DB restore operations
--   • server_branding       — per-server panel theme override (different from reseller branding)
--   • blacklist_alerts      — DNSBL monitoring results (Step 3.11)
--   • reseller_packages     — feature flag packages (Step 4.19)
--   • server_smarthost      — per-server outbound relay config (Step 3.16)

-- ── Backup restore jobs ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS backup_restore_jobs (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    backup_id       UUID         NOT NULL REFERENCES backups(id) ON DELETE CASCADE,
    site_id         UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    requested_by    UUID         NOT NULL REFERENCES users(id),
    restore_type    TEXT         NOT NULL DEFAULT 'full'
                                 CHECK (restore_type IN ('full','files','database','single_file')),
    target_path     TEXT,        -- for single_file restore: relative path within archive
    status          TEXT         NOT NULL DEFAULT 'pending'
                                 CHECK (status IN ('pending','running','complete','failed')),
    error_msg       TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_backup_restore_jobs_site
    ON backup_restore_jobs(site_id, created_at DESC);

-- ── Blacklist alerts ──────────────────────────────────────────────────────────
-- DNSBL check results (Step 3.11). orbit-cron checks every 6 hours.
CREATE TABLE IF NOT EXISTS blacklist_alerts (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id       UUID         REFERENCES servers(id) ON DELETE CASCADE,
    domain          TEXT,        -- domain checked (may be NULL if IP-only check)
    ip              INET         NOT NULL,
    dnsbl           TEXT         NOT NULL, -- "zen.spamhaus.org", "b.barracudacentral.org", etc.
    dnsbl_response  TEXT,        -- returned TXT / A record
    listed_at       TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ,
    notified        BOOLEAN      NOT NULL DEFAULT FALSE,
    UNIQUE (ip, dnsbl, domain)
);

CREATE INDEX IF NOT EXISTS idx_blacklist_alerts_server
    ON blacklist_alerts(server_id, listed_at DESC);
CREATE INDEX IF NOT EXISTS idx_blacklist_alerts_active
    ON blacklist_alerts(ip, dnsbl) WHERE resolved_at IS NULL;

-- ── Reseller packages / feature flags ────────────────────────────────────────
-- Step 4.19: per-package feature toggles checked in client portal.
CREATE TABLE IF NOT EXISTS reseller_packages (
    id                      UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    reseller_id             UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                    TEXT         NOT NULL,    -- "Starter", "Pro", "Enterprise"
    max_sites               INT,         -- NULL = unlimited
    max_disk_gb             INT,
    max_email_accounts      INT,
    max_databases           INT,
    max_cron_jobs           INT,
    feat_dns_editing        BOOLEAN      NOT NULL DEFAULT TRUE,
    feat_php_version_switch BOOLEAN      NOT NULL DEFAULT TRUE,
    feat_app_installer      BOOLEAN      NOT NULL DEFAULT TRUE,
    feat_ssh_terminal       BOOLEAN      NOT NULL DEFAULT FALSE,
    feat_staging            BOOLEAN      NOT NULL DEFAULT FALSE,
    feat_db_manager         BOOLEAN      NOT NULL DEFAULT TRUE,
    feat_file_manager       BOOLEAN      NOT NULL DEFAULT TRUE,
    feat_wildcard_ssl       BOOLEAN      NOT NULL DEFAULT FALSE,
    feat_backup_s3          BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_reseller_packages_reseller
    ON reseller_packages(reseller_id);

-- ── Assign package to user ────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS user_packages (
    user_id     UUID  PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    package_id  UUID  NOT NULL REFERENCES reseller_packages(id) ON DELETE RESTRICT,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── Email smarthost / relay per domain ───────────────────────────────────────
-- Step 3.16: per-domain outbound SMTP relay configuration.
-- orbit-mail generates Postfix transport_maps and relay_credentials from this.
CREATE TABLE IF NOT EXISTS email_smarthosts (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    domain_id       UUID         NOT NULL UNIQUE REFERENCES email_domains(id) ON DELETE CASCADE,
    relay_host      TEXT         NOT NULL,    -- "smtp.sendgrid.net"
    relay_port      INT          NOT NULL DEFAULT 587,
    use_tls         BOOLEAN      NOT NULL DEFAULT TRUE,
    username        TEXT,
    password        TEXT,        -- encrypted at rest
    enabled         BOOLEAN      NOT NULL DEFAULT TRUE,
    last_tested_at  TIMESTAMPTZ,
    last_test_ok    BOOLEAN,
    last_test_error TEXT,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TRIGGER reseller_packages_updated_at
    BEFORE UPDATE ON reseller_packages
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER email_smarthosts_updated_at
    BEFORE UPDATE ON email_smarthosts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
