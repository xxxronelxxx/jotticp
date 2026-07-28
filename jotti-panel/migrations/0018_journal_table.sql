CREATE TABLE IF NOT EXISTS journal (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    op_type     TEXT NOT NULL,
    domain      TEXT NOT NULL,
    payload     JSONB,
    status      TEXT NOT NULL DEFAULT 'pending',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    error_msg   TEXT
);
CREATE INDEX IF NOT EXISTS idx_journal_status ON journal(status, created_at);
