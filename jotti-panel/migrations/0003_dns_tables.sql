-- Migration 0003: DNS supplemental tables
-- dns_zones and dns_records are already created in 0001_initial.sql.
-- This migration adds supplemental tables for DNS management features.

-- ── DNS zone templates ────────────────────────────────────────────────────────
-- Reusable record sets applied when a new zone is created.
-- "default" template adds SOA + NS + MX + SPF + DMARC automatically.
CREATE TABLE IF NOT EXISTS dns_zone_templates (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT         NOT NULL,           -- "Default hosting", "Email only"
    description TEXT,
    records     JSONB        NOT NULL DEFAULT '[]',
    -- [{"name":"@","type":"A","content":"{{server_ip}}","ttl":300}, ...]
    is_default  BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dns_zone_templates_owner
    ON dns_zone_templates(owner_id);

-- ── DNS propagation check cache ───────────────────────────────────────────────
-- Short-lived cache of propagation check results (Step 4.17).
-- Rows expire after 5 minutes — orbit-cron sweeps them hourly.
CREATE TABLE IF NOT EXISTS dns_propagation_checks (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    zone_id     UUID         NOT NULL REFERENCES dns_zones(id) ON DELETE CASCADE,
    record_id   UUID         REFERENCES dns_records(id) ON DELETE CASCADE,
    resolver    INET         NOT NULL,           -- 8.8.8.8, 1.1.1.1, etc.
    resolved    BOOLEAN      NOT NULL DEFAULT FALSE,
    response    TEXT,
    checked_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW() + INTERVAL '5 minutes'
);

CREATE INDEX IF NOT EXISTS idx_dns_propagation_zone
    ON dns_propagation_checks(zone_id, checked_at DESC);

-- Index for live check lookups (can't use NOW() in partial index predicate — not IMMUTABLE)
CREATE INDEX IF NOT EXISTS idx_dns_propagation_live
    ON dns_propagation_checks(zone_id, record_id, expires_at);

-- ── DNSSEC keys (deferred v2.0 — table created now, populated later) ─────────
CREATE TABLE IF NOT EXISTS dns_dnssec_keys (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    zone_id     UUID         NOT NULL REFERENCES dns_zones(id) ON DELETE CASCADE,
    key_tag     INT          NOT NULL,
    algorithm   INT          NOT NULL DEFAULT 13,  -- ECDSA P-256 SHA-256 (RFC 6605)
    key_type    TEXT         NOT NULL CHECK (key_type IN ('KSK','ZSK')),
    public_key  TEXT         NOT NULL,
    private_key TEXT,        -- NULL = external HSM; encrypted at rest when present
    is_active   BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dns_dnssec_zone
    ON dns_dnssec_keys(zone_id) WHERE is_active = TRUE;
