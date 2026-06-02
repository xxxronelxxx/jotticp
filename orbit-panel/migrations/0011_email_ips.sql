-- Step 3.12: Reseller email IP pools
-- email_ip_pools: one row per dedicated IP assigned to a reseller
-- email_ip_assignments: maps individual domains to a pool entry

CREATE TABLE IF NOT EXISTS email_ip_pools (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reseller_id UUID REFERENCES users(id) ON DELETE CASCADE,
    ip_address  INET NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_email_ip_pools_reseller_id
    ON email_ip_pools (reseller_id);

CREATE TABLE IF NOT EXISTS email_ip_assignments (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id     UUID NOT NULL REFERENCES email_ip_pools(id) ON DELETE CASCADE,
    domain      VARCHAR(255) NOT NULL,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (domain)   -- each domain can only have one dedicated IP binding
);

CREATE INDEX IF NOT EXISTS idx_email_ip_assignments_pool_id
    ON email_ip_assignments (pool_id);
