-- Migration 0002: Email supplemental tables
-- email_accounts and email_domains are already in 0001_initial.sql.
-- This migration adds email_forwarders and email_autoresponders as
-- dedicated normalized tables (0001 only had forward_to TEXT inline on
-- email_accounts — these tables allow many-to-one forwarding and richer
-- autoresponder control).
--
-- NOTE: email_accounts already exists with the domain_id / local_part model.
-- The address column variant requested by the API is a computed view only.

-- ── Email forwarders ──────────────────────────────────────────────────────────
-- Separate from email_accounts.forward_to so that catch-all and
-- one-to-many forwarding are supported cleanly.
CREATE TABLE IF NOT EXISTS email_forwarders (
    id            UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    domain_id     UUID         NOT NULL REFERENCES email_domains(id) ON DELETE CASCADE,
    from_address  TEXT         NOT NULL,           -- "sales@example.com" or "@example.com" (catch-all)
    to_address    TEXT         NOT NULL,           -- external or internal address
    enabled       BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS uidx_email_forwarders_from
    ON email_forwarders(from_address);
CREATE INDEX IF NOT EXISTS idx_email_forwarders_domain
    ON email_forwarders(domain_id);

-- ── Email autoresponders ──────────────────────────────────────────────────────
-- Richer autoresponder control beyond the boolean flag in email_accounts.
-- orbit-mail writes Sieve vacation scripts from this table.
CREATE TABLE IF NOT EXISTS email_autoresponders (
    id            UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id    UUID         NOT NULL REFERENCES email_accounts(id) ON DELETE CASCADE,
    subject       TEXT         NOT NULL,
    body          TEXT         NOT NULL,
    start_date    DATE,
    end_date      DATE,
    enabled       BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS uidx_email_autoresponders_account
    ON email_autoresponders(account_id);

-- ── Sieve filter rules ────────────────────────────────────────────────────────
-- Per-account Sieve filter scripts managed through the UI (Step 3.15).
-- The compiled script is uploaded to Dovecot managesieve.
CREATE TABLE IF NOT EXISTS email_sieve_rules (
    id            UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id    UUID         NOT NULL REFERENCES email_accounts(id) ON DELETE CASCADE,
    name          TEXT         NOT NULL,           -- "Move newsletters"
    position      INT          NOT NULL DEFAULT 0, -- execution order
    condition_op  TEXT         NOT NULL DEFAULT 'all' CHECK (condition_op IN ('all','any')),
    conditions    JSONB        NOT NULL DEFAULT '[]',
    -- [{"field":"from","op":"contains","value":"newsletter"}]
    actions       JSONB        NOT NULL DEFAULT '[]',
    -- [{"type":"fileinto","folder":"Newsletters"},{"type":"stop"}]
    enabled       BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_email_sieve_account
    ON email_sieve_rules(account_id, position);

-- ── Per-mailbox spam settings ─────────────────────────────────────────────────
-- rspamd user_settings per mailbox (Step 3.14).
CREATE TABLE IF NOT EXISTS email_spam_settings (
    account_id        UUID    PRIMARY KEY REFERENCES email_accounts(id) ON DELETE CASCADE,
    spam_threshold    NUMERIC(4,1) NOT NULL DEFAULT 5.0 CHECK (spam_threshold BETWEEN 1.0 AND 15.0),
    reject_threshold  NUMERIC(4,1) NOT NULL DEFAULT 12.0 CHECK (reject_threshold BETWEEN 1.0 AND 15.0),
    bayes_enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Trigger: keep updated_at current
CREATE TRIGGER email_autoresponders_updated_at
    BEFORE UPDATE ON email_autoresponders
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER email_sieve_rules_updated_at
    BEFORE UPDATE ON email_sieve_rules
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
