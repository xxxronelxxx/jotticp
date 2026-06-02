-- Plugin marketplace tables (Step 5.6)

CREATE TABLE IF NOT EXISTS plugins (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        TEXT        NOT NULL UNIQUE,
    name        TEXT        NOT NULL,
    version     TEXT        NOT NULL DEFAULT '0.0.0',
    author      TEXT        NOT NULL DEFAULT '',
    description TEXT        NOT NULL DEFAULT '',
    tags        TEXT[]      NOT NULL DEFAULT '{}',
    homepage    TEXT,
    icon_url    TEXT,
    enabled     BOOLEAN     NOT NULL DEFAULT TRUE,
    manifest    JSONB       NOT NULL DEFAULT '{}',
    install_path TEXT,
    requires_plan TEXT       NOT NULL DEFAULT 'community' CHECK (requires_plan IN ('community', 'pro')),
    installed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_plugins_slug    ON plugins (slug);
CREATE INDEX IF NOT EXISTS idx_plugins_enabled ON plugins (enabled);

-- Plugin event subscriptions
CREATE TABLE IF NOT EXISTS plugin_events (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    plugin_id   UUID        NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
    event_name  TEXT        NOT NULL,
    handler     TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (plugin_id, event_name)
);

CREATE INDEX IF NOT EXISTS idx_plugin_events_event ON plugin_events (event_name);
