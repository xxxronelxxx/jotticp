-- Migration 0015: schema fixes for compilation alignment
-- Adds columns discovered missing after 0014 was already applied.

-- dns_zones: status column
ALTER TABLE dns_zones ADD COLUMN IF NOT EXISTS status VARCHAR(32) NOT NULL DEFAULT 'active';

-- dns_records: soft-delete support
ALTER TABLE dns_records ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_dns_records_active ON dns_records(zone_id) WHERE deleted_at IS NULL;

-- email_accounts: full email address alias
ALTER TABLE email_accounts ADD COLUMN IF NOT EXISTS address TEXT;

-- email_forwarders: source/destination aliases for from_address/to_address
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS source TEXT;
ALTER TABLE email_forwarders ADD COLUMN IF NOT EXISTS destination TEXT;
UPDATE email_forwarders SET source = from_address WHERE source IS NULL;
UPDATE email_forwarders SET destination = to_address WHERE destination IS NULL;

-- reseller_packages: migration 0006 created this table with feat_* boolean columns
-- but migration 0007's CREATE TABLE IF NOT EXISTS was a no-op, leaving out price_monthly/feature_flags
ALTER TABLE reseller_packages ADD COLUMN IF NOT EXISTS price_monthly DECIMAL(10,2) NOT NULL DEFAULT 0.00;
ALTER TABLE reseller_packages ADD COLUMN IF NOT EXISTS feature_flags JSONB NOT NULL DEFAULT '{
    "dns_editing": true,
    "php_version_switch": true,
    "app_installer": true,
    "ssh_access": false,
    "staging": false,
    "db_manager": true,
    "file_manager": true
}';
