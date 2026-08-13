ALTER TABLE conversations ADD COLUMN summary TEXT;
ALTER TABLE conversations ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE conversations ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN last_saved_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN pruned_at INTEGER;
ALTER TABLE conversations ADD COLUMN chunk_count INTEGER NOT NULL DEFAULT 0;

UPDATE conversations SET updated_at = created_at WHERE updated_at = 0;
UPDATE conversations SET last_saved_at = created_at WHERE last_saved_at = 0;
UPDATE conversations SET chunk_count = COALESCE(json_array_length(chunks), 0);

CREATE TABLE IF NOT EXISTS images (
  conversation_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  caption TEXT,
  mime TEXT NOT NULL,
  bytes BLOB,
  byte_len INTEGER NOT NULL DEFAULT 0,
  source TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_conversations_parent ON conversations(parent_id);
CREATE INDEX IF NOT EXISTS idx_conversations_updated ON conversations(updated_at);
CREATE INDEX IF NOT EXISTS idx_images_conversation ON images(conversation_id);
