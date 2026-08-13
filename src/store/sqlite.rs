use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{
    decode_chunks, encode_chunks, migrate, now_secs, Backend, Conversation, ImageBlob, ImageMeta,
    ListFilter, Status,
};

pub struct SqliteBackend {
    path: std::path::PathBuf,
}

impl SqliteBackend {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("open sqlite {}", path.display()))?;
        migrate::apply_sqlite(&conn).context("sqlite migrations")?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open(&self.path).with_context(|| format!("open sqlite {}", self.path.display()))
    }
}

impl Backend for SqliteBackend {
    fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let conn = self.connect()?;
        let row = conn
            .query_row(
                "SELECT id, parent_id, title, created_at, latest_message, brief, chunks,
                        summary, status, updated_at, last_saved_at, pruned_at, chunk_count
                 FROM conversations WHERE id = ?1",
                params![id],
                row_to_tuple,
            )
            .optional()
            .context("sqlite load")?;
        match row {
            None => Ok(None),
            Some(tuple) => Ok(Some(tuple_to_conv(tuple)?)),
        }
    }

    fn save(&self, conv: &Conversation) -> Result<()> {
        let conn = self.connect()?;
        let chunks = encode_chunks(&conv.chunks)?;
        conn.execute(
            "INSERT INTO conversations (
                id, parent_id, title, created_at, latest_message, brief, chunks,
                summary, status, updated_at, last_saved_at, pruned_at, chunk_count
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
               parent_id = excluded.parent_id,
               title = excluded.title,
               created_at = excluded.created_at,
               latest_message = excluded.latest_message,
               brief = excluded.brief,
               chunks = excluded.chunks,
               summary = excluded.summary,
               status = excluded.status,
               updated_at = excluded.updated_at,
               last_saved_at = excluded.last_saved_at,
               pruned_at = excluded.pruned_at,
               chunk_count = excluded.chunk_count",
            params![
                conv.id,
                conv.parent_id,
                conv.title,
                conv.created_at as i64,
                conv.latest_message,
                conv.brief,
                chunks,
                conv.summary,
                conv.status.as_str(),
                conv.updated_at as i64,
                conv.last_saved_at as i64,
                conv.pruned_at.map(|v| v as i64),
                conv.chunk_count as i64,
            ],
        )
        .context("sqlite save")?;
        Ok(())
    }

    fn list(&self, filter: &ListFilter) -> Result<Vec<Conversation>> {
        let conn = self.connect()?;
        let include_pruned = if filter.include_pruned { 1i64 } else { 0 };
        let has_cutoff = if filter.older_than_secs.is_some() {
            1i64
        } else {
            0
        };
        let cutoff = filter
            .older_than_secs
            .map(|d| filter.now_secs.saturating_sub(d) as i64)
            .unwrap_or(0);
        let limit = filter.limit.max(1) as i64;
        let mut stmt = conn.prepare(
            "SELECT id, parent_id, title, created_at, latest_message, brief, chunks,
                    summary, status, updated_at, last_saved_at, pruned_at, chunk_count
             FROM conversations
             WHERE (?1 = 1 OR status != 'pruned')
               AND (?2 = 0 OR updated_at < ?3)
             ORDER BY updated_at DESC
             LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(
                params![include_pruned, has_cutoff, cutoff, limit],
                row_to_tuple,
            )
            .context("sqlite list")?;
        let mut out = Vec::new();
        for row in rows {
            let mut conv = tuple_to_conv(row.context("sqlite list row")?)?;
            conv.chunks.clear();
            out.push(conv);
        }
        Ok(out)
    }

    fn prune(&self, id: &str, pruned_at: u64) -> Result<bool> {
        let mut conn = self.connect()?;
        let tx = conn.transaction().context("sqlite prune tx")?;
        let n = tx
            .execute(
                "UPDATE conversations SET
                    chunks = '[]',
                    brief = NULL,
                    status = 'pruned',
                    pruned_at = ?2,
                    updated_at = ?2
                 WHERE id = ?1 AND status != 'pruned'",
                params![id, pruned_at as i64],
            )
            .context("sqlite prune conversation")?;
        tx.execute(
            "UPDATE images SET bytes = NULL WHERE conversation_id = ?1",
            params![id],
        )
        .context("sqlite prune images")?;
        tx.commit().context("sqlite prune commit")?;
        Ok(n > 0)
    }

    fn purge(&self, id: &str) -> Result<bool> {
        let mut conn = self.connect()?;
        let tx = conn.transaction().context("sqlite purge tx")?;
        tx.execute("DELETE FROM images WHERE conversation_id = ?1", params![id])
            .context("sqlite purge images")?;
        let n = tx
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])
            .context("sqlite purge conversation")?;
        tx.commit().context("sqlite purge commit")?;
        Ok(n > 0)
    }

    fn add_image(
        &self,
        conversation_id: &str,
        caption: Option<&str>,
        mime: &str,
        bytes: &[u8],
        byte_len: u64,
        source: Option<&str>,
    ) -> Result<ImageMeta> {
        let mut conn = self.connect()?;
        let tx = conn.transaction().context("sqlite add_image tx")?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM images WHERE conversation_id = ?1",
            params![conversation_id],
            |row| row.get(0),
        )?;
        let created = now_secs() as i64;
        tx.execute(
            "INSERT INTO images (conversation_id, seq, caption, mime, bytes, byte_len, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                conversation_id,
                seq,
                caption,
                mime,
                bytes,
                byte_len as i64,
                source,
                created,
            ],
        )
        .context("sqlite insert image")?;
        tx.commit().context("sqlite add_image commit")?;
        Ok(ImageMeta {
            conversation_id: conversation_id.to_string(),
            seq,
            caption: caption.map(str::to_string),
            mime: mime.to_string(),
            byte_len,
            has_bytes: true,
            source: source.map(str::to_string),
        })
    }

    fn load_image(&self, conversation_id: &str, seq: i64) -> Result<Option<ImageBlob>> {
        let conn = self.connect()?;
        let row = conn
            .query_row(
                "SELECT caption, mime, bytes, byte_len, source
                 FROM images WHERE conversation_id = ?1 AND seq = ?2",
                params![conversation_id, seq],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<Vec<u8>>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .context("sqlite load_image")?;
        match row {
            None => Ok(None),
            Some((caption, mime, bytes, byte_len, source)) => {
                let has_bytes = bytes.as_ref().map(|b| !b.is_empty()).unwrap_or(false);
                Ok(Some(ImageBlob {
                    meta: ImageMeta {
                        conversation_id: conversation_id.to_string(),
                        seq,
                        caption,
                        mime,
                        byte_len: byte_len as u64,
                        has_bytes,
                        source,
                    },
                    bytes: bytes.unwrap_or_default(),
                }))
            }
        }
    }

    fn list_images(&self, conversation_id: &str) -> Result<Vec<ImageMeta>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT seq, caption, mime, bytes IS NOT NULL AND length(bytes) > 0, byte_len, source
             FROM images WHERE conversation_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map(params![conversation_id], |row| {
                Ok(ImageMeta {
                    conversation_id: conversation_id.to_string(),
                    seq: row.get(0)?,
                    caption: row.get(1)?,
                    mime: row.get(2)?,
                    has_bytes: row.get(3)?,
                    byte_len: row.get::<_, i64>(4)? as u64,
                    source: row.get(5)?,
                })
            })
            .context("sqlite list_images")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("sqlite list_images row")?);
        }
        Ok(out)
    }

    fn is_empty(&self) -> Result<bool> {
        let conn = self.connect()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
            .context("sqlite count")?;
        Ok(n == 0)
    }
}

type ConvTuple = (
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
    i64,
    Option<i64>,
    i64,
);

fn row_to_tuple(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConvTuple> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

fn tuple_to_conv(tuple: ConvTuple) -> Result<Conversation> {
    let (
        id,
        parent_id,
        title,
        created_at,
        latest_message,
        brief,
        chunks,
        summary,
        status,
        updated_at,
        last_saved_at,
        pruned_at,
        chunk_count,
    ) = tuple;
    Ok(Conversation {
        id,
        parent_id,
        title,
        created_at: created_at as u64,
        latest_message,
        brief,
        chunks: decode_chunks(&chunks)?,
        summary,
        status: Status::parse(&status),
        updated_at: updated_at as u64,
        last_saved_at: last_saved_at as u64,
        pruned_at: pruned_at.map(|v| v as u64),
        chunk_count: chunk_count as usize,
        images: Vec::new(),
    })
}
