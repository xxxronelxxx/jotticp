-- Step 5.9: Staging environments table
-- One staging environment per production site (enforced by UNIQUE constraint)

CREATE TABLE IF NOT EXISTS staging_sites (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    production_site_id UUID NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    staging_domain    VARCHAR(255) NOT NULL,
    unix_user         VARCHAR(64)  NOT NULL,
    status            VARCHAR(20)  NOT NULL DEFAULT 'creating',
    created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    last_synced_at    TIMESTAMPTZ,
    UNIQUE (production_site_id)  -- one staging per production site
);

CREATE INDEX IF NOT EXISTS idx_staging_sites_production_site_id
    ON staging_sites (production_site_id);
