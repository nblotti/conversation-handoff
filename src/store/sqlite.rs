use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{decode_chunks, encode_chunks, Backend, Conversation};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  parent_id TEXT,
  title TEXT,
  created_at INTEGER NOT NULL,
  latest_message TEXT,
  brief TEXT,
  chunks TEXT NOT NULL DEFAULT '[]'
);
"#;

pub struct SqliteBackend {
    path: std::path::PathBuf,
}

impl SqliteBackend {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open sqlite {}", path.display()))?;
        conn.execute_batch(SCHEMA)
            .context("create sqlite schema")?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open(&self.path)
            .with_context(|| format!("open sqlite {}", self.path.display()))
    }
}

impl Backend for SqliteBackend {
    fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let conn = self.connect()?;
        let row = conn
            .query_row(
                "SELECT id, parent_id, title, created_at, latest_message, brief, chunks
                 FROM conversations WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .context("sqlite load")?;
        match row {
            None => Ok(None),
            Some((id, parent_id, title, created_at, latest_message, brief, chunks)) => {
                Ok(Some(Conversation {
                    id,
                    parent_id,
                    title,
                    created_at: created_at as u64,
                    latest_message,
                    brief,
                    chunks: decode_chunks(&chunks)?,
                }))
            }
        }
    }

    fn save(&self, conv: &Conversation) -> Result<()> {
        let conn = self.connect()?;
        let chunks = encode_chunks(&conv.chunks)?;
        conn.execute(
            "INSERT INTO conversations (id, parent_id, title, created_at, latest_message, brief, chunks)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               parent_id = excluded.parent_id,
               title = excluded.title,
               created_at = excluded.created_at,
               latest_message = excluded.latest_message,
               brief = excluded.brief,
               chunks = excluded.chunks",
            params![
                conv.id,
                conv.parent_id,
                conv.title,
                conv.created_at as i64,
                conv.latest_message,
                conv.brief,
                chunks,
            ],
        )
        .context("sqlite save")?;
        Ok(())
    }
}
