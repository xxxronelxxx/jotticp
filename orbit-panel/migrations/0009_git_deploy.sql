-- Migration 0009: Git push-to-deploy support
-- Adds git_deploy_enabled + git_repo_path columns to sites,
-- and creates a git_deploys history table.

ALTER TABLE sites
    ADD COLUMN IF NOT EXISTS git_deploy_enabled BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS git_repo_path      TEXT;

CREATE TABLE IF NOT EXISTS git_deploys (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id      UUID        NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    commit_hash  VARCHAR(40) NOT NULL,
    commit_msg   TEXT,
    deployed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deployed_by  VARCHAR(100) NOT NULL DEFAULT 'git-hook'
);

CREATE INDEX IF NOT EXISTS git_deploys_site_id_deployed_at
    ON git_deploys (site_id, deployed_at DESC);
