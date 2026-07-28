-- Migration 0012: user_databases table
-- Replaces the lightweight `databases` table from 0001_initial.sql for the
-- orbit-panel databases.rs / dbmanager.rs / backup.rs / jobs.rs code paths.
--
-- Differences from the original `databases` table:
--   • db_type is TEXT (not the db_type ENUM) so new engine names can be added
--     without schema migrations (valkey, surrealdb, ferretdb).
--   • status TEXT tracks provisioning lifecycle: provisioning | active | deleting | error
--   • deleted_at for soft-delete (existing sites.deleted_at pattern).
--
-- The original `databases` table (from 0001_initial.sql) is kept for any
-- code that may reference it directly via the ENUM column.  If you wish to
-- consolidate them, drop `databases` and rename `user_databases` in a future
-- migration after verifying no foreign-key consumers remain.

CREATE TABLE IF NOT EXISTS user_databases (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id     UUID         NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    db_type     TEXT         NOT NULL,   -- "mysql" | "postgres" | "ferretdb" | "valkey" | "surrealdb"
    db_name     VARCHAR(64)  NOT NULL,
    db_user     VARCHAR(64)  NOT NULL DEFAULT '',
    status      TEXT         NOT NULL DEFAULT 'provisioning',
        -- provisioning | active | deleting | error
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    deleted_at  TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_user_databases_site_id
    ON user_databases (site_id)
    WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_user_databases_site_db_name
    ON user_databases (site_id, db_name)
    WHERE deleted_at IS NULL;
