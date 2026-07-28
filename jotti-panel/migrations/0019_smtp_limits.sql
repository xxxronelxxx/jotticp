-- SMTP sending limits per domain
CREATE TABLE smtp_domain_limits (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id         UUID        REFERENCES sites(id) ON DELETE CASCADE,
    domain          VARCHAR(253) NOT NULL,
    hourly_limit    INTEGER     NOT NULL DEFAULT 500,
    daily_limit     INTEGER     NOT NULL DEFAULT 5000,
    rate_action     VARCHAR(20) NOT NULL DEFAULT 'queue' CHECK (rate_action IN ('queue', 'reject', 'bounce')),
    relay_enabled   BOOLEAN     NOT NULL DEFAULT true,
    require_spf     BOOLEAN     NOT NULL DEFAULT false,
    require_dkim    BOOLEAN     NOT NULL DEFAULT false,
    enabled         BOOLEAN     NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(domain)
);

-- Hourly sending counters (reset every hour via cron)
CREATE TABLE smtp_send_counts (
    domain          VARCHAR(253) NOT NULL,
    hour_bucket     TIMESTAMPTZ NOT NULL DEFAULT date_trunc('hour', NOW()),
    sent_count      INTEGER     NOT NULL DEFAULT 0,
    day_bucket      DATE        NOT NULL DEFAULT CURRENT_DATE,
    day_count       INTEGER     NOT NULL DEFAULT 0,
    PRIMARY KEY(domain, hour_bucket)
);

CREATE INDEX idx_smtp_send_counts_domain ON smtp_send_counts(domain);
