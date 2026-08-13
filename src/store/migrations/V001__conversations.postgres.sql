CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  parent_id TEXT,
  title TEXT,
  created_at BIGINT NOT NULL,
  latest_message TEXT,
  brief TEXT,
  chunks TEXT NOT NULL DEFAULT '[]'
);
