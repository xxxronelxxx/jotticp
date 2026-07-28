-- Migration 0021: Add tags column to sites for labelling/filtering
ALTER TABLE sites ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT '{}';
CREATE INDEX IF NOT EXISTS idx_sites_tags ON sites USING gin(tags);
