-- 0026: make the email_accounts (domain_id, local_part) uniqueness soft-delete aware.
--
-- The API enforces uniqueness with `WHERE deleted_at IS NULL`, but the original
-- constraint was global, so soft-deleting then re-creating the same address 500'd
-- on the INSERT. Replace the global UNIQUE constraint with a PARTIAL unique index
-- scoped to non-deleted rows. (Applied live on 192.168.0.79 2026-05-29.)

ALTER TABLE email_accounts DROP CONSTRAINT IF EXISTS email_accounts_domain_id_local_part_key;

CREATE UNIQUE INDEX IF NOT EXISTS email_accounts_domain_local_active_key
    ON email_accounts (domain_id, local_part)
    WHERE deleted_at IS NULL;
