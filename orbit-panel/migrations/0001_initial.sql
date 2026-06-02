-- OrbitCP PostgreSQL 17 Schema
-- Version: 1.0.0
-- Applied automatically by sqlx migrate on startup.

-- ── Extensions ───────────────────────────────────────────────────────────────
CREATE EXTENSION IF NOT EXISTS "pgcrypto";   -- gen_random_uuid()
CREATE EXTENSION IF NOT EXISTS "citext";     -- case-insensitive text (email)

-- ── ENUM types ────────────────────────────────────────────────────────────────
CREATE TYPE user_role AS ENUM ('admin', 'reseller', 'user');
CREATE TYPE site_status AS ENUM ('provisioning', 'active', 'suspended', 'deleting', 'error');
CREATE TYPE ssl_challenge_type AS ENUM ('http-01', 'dns-01');
CREATE TYPE ssl_cert_status AS ENUM ('active', 'pending', 'expired', 'failed', 'revoked');
CREATE TYPE db_type AS ENUM ('mysql', 'mariadb', 'postgresql', 'ferretdb', 'surrealdb');
CREATE TYPE backup_status AS ENUM ('pending', 'running', 'completed', 'failed', 'expired');
CREATE TYPE backup_type AS ENUM ('full', 'files_only', 'db_only', 'pre_app_update');
CREATE TYPE notification_type AS ENUM (
    'ssl_expiring_soon', 'ssl_expired', 'ssl_renewal_failed',
    'disk_usage_high', 'disk_usage_critical',
    'backup_failed', 'backup_succeeded',
    'new_admin_login', 'suspicious_login',
    'app_update_available', 'malware_threat',
    'server_offline', 'server_recovered',
    'site_down', 'site_recovered',
    'ct_alert'
);

-- ── Users ─────────────────────────────────────────────────────────────────────
CREATE TABLE users (
    id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    email               CITEXT       NOT NULL UNIQUE,
    password_hash       TEXT         NOT NULL,
    role                user_role    NOT NULL DEFAULT 'user',
    display_name        VARCHAR(128),
    totp_secret         TEXT,                              -- NULL = TOTP not configured
    totp_backup_codes   TEXT[],                            -- bcrypt hashes of backup codes
    wizard_complete     BOOLEAN      NOT NULL DEFAULT false,
    failed_login_count  INT          NOT NULL DEFAULT 0,
    locked_until        TIMESTAMPTZ,
    last_login_at       TIMESTAMPTZ,
    last_login_ip       INET,
    reseller_id         UUID         REFERENCES users(id) ON DELETE SET NULL,  -- NULL = not a reseller client
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    deleted_at          TIMESTAMPTZ                        -- soft delete
);

CREATE INDEX idx_users_email    ON users(email) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_reseller ON users(reseller_id) WHERE deleted_at IS NULL;

-- ── Servers ───────────────────────────────────────────────────────────────────
CREATE TABLE servers (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    label           VARCHAR(128) NOT NULL,          -- "Web01 — Frankfurt"
    hostname        VARCHAR(255) NOT NULL,
    ip              INET         NOT NULL,
    ssh_port        INT          NOT NULL DEFAULT 22,
    ssh_user        VARCHAR(64)  NOT NULL DEFAULT 'orbitcp-admin',
    agent_port      INT          NOT NULL DEFAULT 7443,  -- orbit-agent gRPC port
    agent_cert      TEXT,                               -- mTLS client cert (PEM)
    agent_key       TEXT,                               -- mTLS client key  (PEM, encrypted at rest)
    os_version      VARCHAR(128),
    kernel_version  VARCHAR(128),
    ram_total_mb    BIGINT,
    cpu_count       INT,
    disk_total_gb   BIGINT,
    owner_id        UUID         NOT NULL REFERENCES users(id),
    status          VARCHAR(32)  NOT NULL DEFAULT 'active',  -- active | offline | error
    last_seen_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ
);

CREATE INDEX idx_servers_owner ON servers(owner_id) WHERE deleted_at IS NULL;

-- ── Sites ─────────────────────────────────────────────────────────────────────
CREATE TABLE sites (
    id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    domain              VARCHAR(253) NOT NULL,
    status              site_status  NOT NULL DEFAULT 'provisioning',
    php_version         VARCHAR(8)   NOT NULL DEFAULT '8.3',
    web_server          VARCHAR(16)  NOT NULL DEFAULT 'ols',  -- ols|nginx|apache|lse
    unix_user           VARCHAR(64)  NOT NULL,
    server_id           UUID         REFERENCES servers(id) ON DELETE SET NULL,
    owner_id            UUID         NOT NULL REFERENCES users(id),
    disk_mb             BIGINT       NOT NULL DEFAULT 0,
    disk_quota_mb       BIGINT,                                  -- NULL = unlimited
    bandwidth_quota_gb  BIGINT,
    document_root       TEXT,                                    -- /home/orbit_example/htdocs
    php_fpm_socket      TEXT,                                    -- /run/php/orbit_example.sock
    cgroup_slice        TEXT,                                    -- website-orbit_example.slice
    provisioning_error  TEXT,
    suspended_at        TIMESTAMPTZ,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ,
    deleted_at          TIMESTAMPTZ
);

-- Domain must be unique across non-deleted sites
CREATE UNIQUE INDEX idx_sites_domain ON sites(domain) WHERE deleted_at IS NULL;
CREATE INDEX idx_sites_server  ON sites(server_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_sites_owner   ON sites(owner_id)  WHERE deleted_at IS NULL;
CREATE INDEX idx_sites_status  ON sites(status)    WHERE deleted_at IS NULL;

-- Junction table: users who have access to a site (owner + granted users)
CREATE TABLE site_users (
    site_id    UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    user_id    UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role       VARCHAR(16)  NOT NULL DEFAULT 'owner',  -- owner | manager | viewer
    granted_at TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    PRIMARY KEY (site_id, user_id)
);

-- ── Databases (hosted) ────────────────────────────────────────────────────────
CREATE TABLE databases (
    id              UUID      PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID      REFERENCES sites(id) ON DELETE CASCADE,
    server_id       UUID      REFERENCES servers(id) ON DELETE SET NULL,
    owner_id        UUID      NOT NULL REFERENCES users(id),
    db_type         db_type   NOT NULL DEFAULT 'mysql',
    db_name         VARCHAR(64)  NOT NULL,
    db_user         VARCHAR(64)  NOT NULL,
    db_host         INET         NOT NULL DEFAULT '127.0.0.1',
    db_port         INT          NOT NULL DEFAULT 3306,
    size_mb         BIGINT       NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ,
    deleted_at      TIMESTAMPTZ
);

CREATE INDEX idx_databases_site  ON databases(site_id)  WHERE deleted_at IS NULL;
CREATE INDEX idx_databases_owner ON databases(owner_id) WHERE deleted_at IS NULL;

-- ── Email ─────────────────────────────────────────────────────────────────────
CREATE TABLE email_domains (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    domain          VARCHAR(253) NOT NULL UNIQUE,
    site_id         UUID         REFERENCES sites(id) ON DELETE CASCADE,
    server_id       UUID         REFERENCES servers(id) ON DELETE SET NULL,
    dkim_selector   VARCHAR(64)  NOT NULL DEFAULT 'orbit',
    dkim_public_key TEXT,
    dkim_private_key TEXT,       -- encrypted at rest
    spf_record      TEXT,
    dmarc_record    TEXT,
    mail_server_ip  INET,
    status          VARCHAR(16)  NOT NULL DEFAULT 'active',
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TABLE email_accounts (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    domain_id       UUID         NOT NULL REFERENCES email_domains(id) ON DELETE CASCADE,
    local_part      VARCHAR(64)  NOT NULL,   -- "admin" (full: admin@domain.com)
    password_hash   TEXT         NOT NULL,
    quota_mb        INT          NOT NULL DEFAULT 1024,
    used_mb         INT          NOT NULL DEFAULT 0,
    forward_to      TEXT,                    -- NULL = normal mailbox
    is_alias        BOOLEAN      NOT NULL DEFAULT false,
    autoresponder_enabled BOOLEAN NOT NULL DEFAULT false,
    autoresponder_message TEXT,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ,
    UNIQUE (domain_id, local_part)
);

CREATE INDEX idx_email_accounts_domain ON email_accounts(domain_id) WHERE deleted_at IS NULL;

-- ── DNS ───────────────────────────────────────────────────────────────────────
CREATE TABLE dns_zones (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    zone        VARCHAR(253) NOT NULL UNIQUE,  -- "example.com."
    site_id     UUID         REFERENCES sites(id) ON DELETE SET NULL,
    server_id   UUID         REFERENCES servers(id) ON DELETE SET NULL,
    owner_id    UUID         NOT NULL REFERENCES users(id),
    serial      BIGINT       NOT NULL DEFAULT 2026010100,
    pdns_zone_id VARCHAR(64),    -- PowerDNS internal zone ID
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ
);

CREATE TABLE dns_records (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    zone_id     UUID         NOT NULL REFERENCES dns_zones(id) ON DELETE CASCADE,
    name        VARCHAR(253) NOT NULL,   -- "@", "www", "mail", etc.
    type        VARCHAR(16)  NOT NULL,   -- A, AAAA, MX, TXT, CNAME, NS, SRV, CAA
    content     TEXT         NOT NULL,
    ttl         INT          NOT NULL DEFAULT 3600,
    priority    INT,                     -- MX, SRV priority
    disabled    BOOLEAN      NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ
);

CREATE INDEX idx_dns_records_zone ON dns_records(zone_id);
CREATE INDEX idx_dns_records_type ON dns_records(zone_id, type);

-- ── SSL Certificates ──────────────────────────────────────────────────────────
CREATE TABLE ssl_certs (
    id                  UUID              PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id             UUID              REFERENCES sites(id) ON DELETE CASCADE,
    domain              VARCHAR(253)      NOT NULL,
    issuer              VARCHAR(255),
    status              ssl_cert_status   NOT NULL DEFAULT 'pending',
    challenge_type      ssl_challenge_type,
    issued_at           TIMESTAMPTZ,
    expires_at          TIMESTAMPTZ,
    cert_path           TEXT,             -- /etc/orbitcp/ssl/<domain>/fullchain.pem
    key_path            TEXT,
    chain_path          TEXT,
    auto_renew          BOOLEAN           NOT NULL DEFAULT true,
    last_renewal_attempt TIMESTAMPTZ,
    last_renewal_error  TEXT,
    created_at          TIMESTAMPTZ       NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_ssl_certs_site   ON ssl_certs(site_id);
CREATE INDEX idx_ssl_certs_expiry ON ssl_certs(expires_at) WHERE auto_renew = true AND status = 'active';

-- ── Backups ───────────────────────────────────────────────────────────────────
CREATE TABLE backup_schedules (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID         REFERENCES sites(id) ON DELETE CASCADE,
    server_id       UUID         REFERENCES servers(id) ON DELETE CASCADE,
    owner_id        UUID         NOT NULL REFERENCES users(id),
    cron_expression VARCHAR(64)  NOT NULL DEFAULT '0 3 * * *',  -- daily at 03:00
    retention_days  INT          NOT NULL DEFAULT 30,
    storage_type    VARCHAR(16)  NOT NULL DEFAULT 'local',       -- local | s3 | b2 | sftp
    storage_config  JSONB,       -- encrypted storage credentials
    include_db      BOOLEAN      NOT NULL DEFAULT true,
    include_files   BOOLEAN      NOT NULL DEFAULT true,
    enabled         BOOLEAN      NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TABLE backups (
    id              UUID          PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id     UUID          REFERENCES backup_schedules(id) ON DELETE SET NULL,
    site_id         UUID          REFERENCES sites(id) ON DELETE CASCADE,
    server_id       UUID          REFERENCES servers(id) ON DELETE SET NULL,
    backup_type     backup_type   NOT NULL DEFAULT 'full',
    status          backup_status NOT NULL DEFAULT 'pending',
    storage_path    TEXT,         -- local path or S3/B2 key
    size_bytes      BIGINT,
    checksum_sha256 TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ,
    error_message   TEXT,
    created_at      TIMESTAMPTZ   NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_backups_site   ON backups(site_id, created_at DESC);
CREATE INDEX idx_backups_server ON backups(server_id, created_at DESC);
CREATE INDEX idx_backups_status ON backups(status) WHERE status IN ('pending', 'running');

-- ── Resellers ─────────────────────────────────────────────────────────────────
CREATE TABLE resellers (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID         NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    plan        VARCHAR(16)  NOT NULL DEFAULT 'pro',
    max_servers INT,                   -- NULL = unlimited
    max_sites   INT,                   -- NULL = unlimited
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TABLE reseller_brands (
    id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    reseller_id         UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    brand_name          VARCHAR(128) NOT NULL,
    panel_domain        VARCHAR(255),
    logo_path           TEXT,
    favicon_path        TEXT,
    primary_color       VARCHAR(7),     -- "#1a56db"
    accent_color        VARCHAR(7),
    support_email       VARCHAR(255),
    support_url         TEXT,
    hide_orbitcp_credit BOOLEAN      NOT NULL DEFAULT false,
    custom_css          TEXT,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- ── App Installs (one-click installer) ────────────────────────────────────────
CREATE TABLE app_installs (
    id                UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id           UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    manifest_slug     VARCHAR(64)  NOT NULL,          -- "wordpress"
    installed_version VARCHAR(32)  NOT NULL,
    latest_version    VARCHAR(32),
    db_name           VARCHAR(64),
    db_user           VARCHAR(64),
    install_path      TEXT         NOT NULL,           -- absolute path
    status            VARCHAR(16)  NOT NULL DEFAULT 'active',  -- active|updating|failed|removed
    installed_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ,
    metadata          JSONB
);

CREATE INDEX idx_app_installs_site ON app_installs(site_id);

-- ── API Keys ──────────────────────────────────────────────────────────────────
CREATE TABLE api_keys (
    id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id             UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                VARCHAR(128) NOT NULL,          -- "WHMCS Production"
    key_hash            TEXT         NOT NULL,           -- SHA-256 hash — never stored plaintext
    key_prefix          VARCHAR(16)  NOT NULL,           -- "sk_live_AbCdEfGh" (for identification)
    scopes              TEXT[]       NOT NULL DEFAULT '{}',
    rate_limit_per_min  INT          NOT NULL DEFAULT 60,
    last_used_at        TIMESTAMPTZ,
    last_used_ip        INET,
    expires_at          TIMESTAMPTZ,
    is_active           BOOLEAN      NOT NULL DEFAULT true,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_api_keys_user      ON api_keys(user_id)   WHERE is_active = true;
CREATE INDEX idx_api_keys_prefix    ON api_keys(key_prefix) WHERE is_active = true;

-- ── Audit Log ─────────────────────────────────────────────────────────────────
-- BIGSERIAL for high-volume inserts — not UUID
-- Partitioned by month; PK must include partition key (PostgreSQL requirement)
CREATE SEQUENCE audit_log_id_seq;

CREATE TABLE audit_log (
    id              BIGINT       NOT NULL DEFAULT nextval('audit_log_id_seq'),
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    user_id         UUID         REFERENCES users(id) ON DELETE SET NULL,
    api_key_id      UUID         REFERENCES api_keys(id) ON DELETE SET NULL,
    ip_address      INET         NOT NULL,
    user_agent      TEXT,
    action          VARCHAR(128) NOT NULL,     -- "site.create", "user.login", etc.
    target_type     VARCHAR(64),               -- "site", "user", "database", ...
    target_id       UUID,
    target_name     VARCHAR(255),              -- human-readable: "example.com"
    request_path    TEXT,
    request_method  VARCHAR(8),
    status_code     INT,
    duration_ms     INT,
    details         JSONB,                     -- sanitized request body
    success         BOOLEAN      NOT NULL DEFAULT true,
    error_msg       TEXT,
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

-- Monthly partitions (created dynamically by orbit-cron)
CREATE TABLE audit_log_y2026m01 PARTITION OF audit_log
    FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
CREATE TABLE audit_log_y2026m05 PARTITION OF audit_log
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE audit_log_y2026m06 PARTITION OF audit_log
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');

CREATE INDEX idx_audit_log_user   ON audit_log(user_id, created_at DESC);
CREATE INDEX idx_audit_log_action ON audit_log(action, created_at DESC);
CREATE INDEX idx_audit_log_target ON audit_log(target_type, target_id);
CREATE INDEX idx_audit_log_ip     ON audit_log(ip_address, created_at DESC);

-- ── Cron Jobs ─────────────────────────────────────────────────────────────────
CREATE TABLE cron_jobs (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID         REFERENCES sites(id) ON DELETE CASCADE,
    owner_id        UUID         NOT NULL REFERENCES users(id),
    label           VARCHAR(128),
    command         TEXT         NOT NULL,      -- the actual command to run
    schedule        VARCHAR(64)  NOT NULL,      -- cron expression: "0 3 * * *"
    schedule_human  VARCHAR(128),               -- "Every day at 3:00 AM"
    unix_user       VARCHAR(64),                -- run as this user
    enabled         BOOLEAN      NOT NULL DEFAULT true,
    last_run_at     TIMESTAMPTZ,
    last_run_status VARCHAR(16),                -- success | failed | running
    last_run_output TEXT,
    next_run_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cron_jobs_site ON cron_jobs(site_id);

-- ── Resource Snapshots ────────────────────────────────────────────────────────
-- Taken every 15 minutes by orbit-cron for usage reports and billing
CREATE TABLE resource_snapshots (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    server_id       UUID         NOT NULL REFERENCES servers(id),
    snapshot_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    disk_bytes      BIGINT       NOT NULL DEFAULT 0,
    db_bytes        BIGINT       NOT NULL DEFAULT 0,
    cpu_ns          BIGINT       NOT NULL DEFAULT 0,   -- cgroup cpu.stat usage_usec * 1000
    memory_bytes    BIGINT       NOT NULL DEFAULT 0,   -- cgroup memory.current
    bandwidth_in_bytes  BIGINT   NOT NULL DEFAULT 0,
    bandwidth_out_bytes BIGINT   NOT NULL DEFAULT 0,
    request_count   BIGINT       NOT NULL DEFAULT 0
);

CREATE INDEX idx_resource_snapshots_site_time
    ON resource_snapshots(site_id, snapshot_at DESC);

-- ── Notifications ─────────────────────────────────────────────────────────────
CREATE TABLE notifications (
    id          UUID              PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID              REFERENCES users(id) ON DELETE CASCADE,
    type        notification_type NOT NULL,
    title       VARCHAR(255)      NOT NULL,
    body        TEXT              NOT NULL,
    action_url  TEXT,
    read_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ       NOT NULL DEFAULT NOW(),
    site_id     UUID              REFERENCES sites(id) ON DELETE SET NULL,
    server_id   UUID              REFERENCES servers(id) ON DELETE SET NULL,
    metadata    JSONB
);

CREATE INDEX idx_notifications_user_unread
    ON notifications(user_id, created_at DESC) WHERE read_at IS NULL;

-- ── Functions and Triggers ────────────────────────────────────────────────────

-- Auto-update updated_at columns
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER sites_updated_at
    BEFORE UPDATE ON sites
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER servers_updated_at
    BEFORE UPDATE ON servers
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
