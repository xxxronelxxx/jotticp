-- Migration 0024: Cloudflare API tokens for DNS integration
CREATE TABLE IF NOT EXISTS cloudflare_tokens (
    id             UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    label          VARCHAR(128) NOT NULL,
    api_token_enc  TEXT         NOT NULL,    -- XOR-encrypted API token
    zone_id        VARCHAR(64)  NOT NULL,    -- Cloudflare zone ID (32-char hex)
    domain         VARCHAR(253) NOT NULL,    -- Apex domain this zone covers
    proxy          BOOLEAN      NOT NULL DEFAULT false,
    created_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_cloudflare_tokens_domain ON cloudflare_tokens(domain);
