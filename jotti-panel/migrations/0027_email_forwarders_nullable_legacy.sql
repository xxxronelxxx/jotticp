-- 0027: the forwarder API inserts (source,destination); the legacy NOT-NULL columns
-- (domain_id, from_address, to_address) blocked inserts. Make them nullable; the
-- Postfix virtual_alias map COALESCEs source/from_address + destination/to_address.
ALTER TABLE email_forwarders ALTER COLUMN domain_id DROP NOT NULL;
ALTER TABLE email_forwarders ALTER COLUMN from_address DROP NOT NULL;
ALTER TABLE email_forwarders ALTER COLUMN to_address DROP NOT NULL;
