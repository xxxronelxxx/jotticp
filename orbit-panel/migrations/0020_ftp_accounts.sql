-- Migration 0020: FTP virtual user accounts
-- Each FTP account belongs to a site, authenticates via pure-ftpd PureDB,
-- and is chrooted to a subdirectory of the site's document root.

CREATE TABLE IF NOT EXISTS ftp_accounts (
    id           UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id      UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    username     VARCHAR(64)  NOT NULL,
    -- SHA-512 crypt hash of password (compatible with pure-ftpd's UnixAuthentication)
    password_hash TEXT        NOT NULL,
    -- Path relative to site docroot, e.g. "/" or "/uploads"
    home_subdir  TEXT         NOT NULL DEFAULT '/',
    quota_mb     INTEGER      NOT NULL DEFAULT 0,  -- 0 = unlimited
    enabled      BOOLEAN      NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    -- username must be unique across all sites (FTP login namespace is global)
    UNIQUE (username)
);

CREATE INDEX IF NOT EXISTS idx_ftp_accounts_site ON ftp_accounts(site_id);
