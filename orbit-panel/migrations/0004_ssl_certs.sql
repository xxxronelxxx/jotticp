-- Migration 0004: SSL supplemental tables
-- ssl_certs is already created in 0001_initial.sql with the full column set.
-- This migration adds the ACME order tracking table and custom cert upload table
-- needed by the instant-acme integration (Step 2.8).

-- ── ACME orders ───────────────────────────────────────────────────────────────
-- Tracks in-flight ACME v2 order state so that the panel can resume
-- challenge verification after a restart.
CREATE TABLE IF NOT EXISTS acme_orders (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    ssl_cert_id     UUID         NOT NULL REFERENCES ssl_certs(id) ON DELETE CASCADE,
    site_id         UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    order_url       TEXT         NOT NULL,           -- ACME order URL at Let's Encrypt
    status          TEXT         NOT NULL DEFAULT 'pending'
                                 CHECK (status IN ('pending','ready','processing','valid','invalid')),
    challenge_type  TEXT         NOT NULL DEFAULT 'http-01'
                                 CHECK (challenge_type IN ('http-01','dns-01')),
    challenge_token TEXT,        -- HTTP-01 token placed under /.well-known/acme-challenge/
    challenge_key   TEXT,        -- key authorization value
    dns_txt_record  TEXT,        -- DNS-01 TXT value (_acme-challenge.domain)
    finalize_url    TEXT,
    certificate_url TEXT,
    account_key_pem TEXT,        -- ACME account key (PEM, encrypted at rest)
    error_detail    TEXT,
    expires_at      TIMESTAMPTZ, -- order expiry per RFC 8555 §7.1.6
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_acme_orders_site
    ON acme_orders(site_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_acme_orders_status
    ON acme_orders(status) WHERE status IN ('pending','ready','processing');

-- ── Custom certificate uploads ────────────────────────────────────────────────
-- Pro plan: users can upload custom certs.
-- Separate table so auto-renew logic never touches custom certs by mistake.
CREATE TABLE IF NOT EXISTS ssl_custom_certs (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    ssl_cert_id     UUID         NOT NULL UNIQUE REFERENCES ssl_certs(id) ON DELETE CASCADE,
    uploaded_by     UUID         NOT NULL REFERENCES users(id),
    cert_pem        TEXT         NOT NULL,           -- full chain PEM
    key_pem         TEXT         NOT NULL,           -- encrypted at rest
    chain_pem       TEXT,
    fingerprint_sha256 TEXT,
    uploaded_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TRIGGER acme_orders_updated_at
    BEFORE UPDATE ON acme_orders
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
