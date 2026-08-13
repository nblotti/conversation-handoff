ALTER TABLE conversations ADD COLUMN IF NOT EXISTS owner TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_conversations_owner ON conversations(owner);
