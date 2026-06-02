-- Migration 0008: SSH keys table for admin SSH key management (Step 6.4)

CREATE TABLE IF NOT EXISTS ssh_keys (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label       VARCHAR(100),
    public_key  TEXT         NOT NULL,
    fingerprint VARCHAR(100) NOT NULL,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ssh_keys_user
    ON ssh_keys(user_id, created_at DESC);

-- Prevent duplicate keys per user
CREATE UNIQUE INDEX IF NOT EXISTS idx_ssh_keys_user_fingerprint
    ON ssh_keys(user_id, fingerprint);
