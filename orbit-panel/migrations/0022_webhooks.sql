-- Migration 0022: Webhook endpoints for event notifications
CREATE TABLE IF NOT EXISTS webhooks (
    id         UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    name       VARCHAR(128) NOT NULL,
    url        TEXT         NOT NULL,
    events     TEXT[]       NOT NULL DEFAULT '{}',
    secret     TEXT,                        -- HMAC-SHA256 signing secret (optional)
    enabled    BOOLEAN      NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_webhooks_events ON webhooks USING gin(events);
CREATE INDEX IF NOT EXISTS idx_webhooks_enabled ON webhooks(enabled) WHERE enabled = true;
