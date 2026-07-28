-- Migration 0023: Admin IP allowlist (restrict admin panel access by IP/CIDR)

-- Dedicated table so we don't abuse site_settings with a fake site_id
CREATE TABLE IF NOT EXISTS panel_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL DEFAULT ''
);

-- Admin IP allowlist: comma-separated CIDR list (empty = allow all)
INSERT INTO panel_settings (key, value) VALUES ('admin_ip_allowlist', '')
ON CONFLICT (key) DO NOTHING;
