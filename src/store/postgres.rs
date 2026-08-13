use std::sync::Mutex;

use anyhow::{Context, Result};
use postgres::types::ToSql;
use postgres::{Client, NoTls, Row};

use crate::config::StoreConfig;

use super::{
    decode_chunks, encode_chunks, migrate, now_secs, Backend, Conversation, ImageBlob, ImageMeta,
    ListFilter, Status,
};

pub struct PostgresBackend {
    client: Mutex<Client>,
}

impl PostgresBackend {
    pub fn connect(cfg: &StoreConfig) -> Result<Self> {
        let mut client = connect(cfg)?;
        migrate::apply_postgres(&mut client).context("postgres migrations")?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }
}

impl Backend for PostgresBackend {
    fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let mut client = lock(&self.client)?;
        let row = client
            .query_opt(
                "SELECT id, parent_id, title, created_at, latest_message, brief, chunks,
                        summary, status, updated_at, last_saved_at, pruned_at, chunk_count
                 FROM conversations WHERE id = $1",
                &[&id],
            )
            .context("postgres load")?;
        match row {
            None => Ok(None),
            Some(row) => Ok(Some(row_to_conv(&row)?)),
        }
    }

    fn save(&self, conv: &Conversation) -> Result<()> {
        let chunks = encode_chunks(&conv.chunks)?;
        let created = conv.created_at as i64;
        let updated = conv.updated_at as i64;
        let last_saved = conv.last_saved_at as i64;
        let pruned = conv.pruned_at.map(|v| v as i64);
        let chunk_count = conv.chunk_count as i64;
        let status = conv.status.as_str();
        let mut client = lock(&self.client)?;
        let params: [&(dyn ToSql + Sync); 13] = [
            &conv.id,
            &conv.parent_id,
            &conv.title,
            &created,
            &conv.latest_message,
            &conv.brief,
            &chunks,
            &conv.summary,
            &status,
            &updated,
            &last_saved,
            &pruned,
            &chunk_count,
        ];
        client
            .execute(
                "INSERT INTO conversations (
                    id, parent_id, title, created_at, latest_message, brief, chunks,
                    summary, status, updated_at, last_saved_at, pruned_at, chunk_count
                 )
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT (id) DO UPDATE SET
                   parent_id = EXCLUDED.parent_id,
                   title = EXCLUDED.title,
                   created_at = EXCLUDED.created_at,
                   latest_message = EXCLUDED.latest_message,
                   brief = EXCLUDED.brief,
                   chunks = EXCLUDED.chunks,
                   summary = EXCLUDED.summary,
                   status = EXCLUDED.status,
                   updated_at = EXCLUDED.updated_at,
                   last_saved_at = EXCLUDED.last_saved_at,
                   pruned_at = EXCLUDED.pruned_at,
                   chunk_count = EXCLUDED.chunk_count",
                &params,
            )
            .context("postgres save")?;
        Ok(())
    }

    fn list(&self, filter: &ListFilter) -> Result<Vec<Conversation>> {
        let include_pruned = filter.include_pruned;
        let has_cutoff = filter.older_than_secs.is_some();
        let cutoff = filter
            .older_than_secs
            .map(|d| filter.now_secs.saturating_sub(d) as i64)
            .unwrap_or(0);
        let limit = filter.limit.max(1) as i64;
        let mut client = lock(&self.client)?;
        let rows = client
            .query(
                "SELECT id, parent_id, title, created_at, latest_message, brief, chunks,
                        summary, status, updated_at, last_saved_at, pruned_at, chunk_count
                 FROM conversations
                 WHERE ($1 OR status != 'pruned')
                   AND (NOT $2 OR updated_at < $3)
                 ORDER BY updated_at DESC
                 LIMIT $4",
                &[&include_pruned, &has_cutoff, &cutoff, &limit],
            )
            .context("postgres list")?;
        let mut out = Vec::new();
        for row in rows {
            let mut conv = row_to_conv(&row)?;
            conv.chunks.clear();
            out.push(conv);
        }
        Ok(out)
    }

    fn prune(&self, id: &str, pruned_at: u64) -> Result<bool> {
        let pruned_at = pruned_at as i64;
        let mut client = lock(&self.client)?;
        let n = client
            .execute(
                "UPDATE conversations SET
                    chunks = '[]',
                    brief = NULL,
                    status = 'pruned',
                    pruned_at = $2,
                    updated_at = $2
                 WHERE id = $1 AND status != 'pruned'",
                &[&id, &pruned_at],
            )
            .context("postgres prune conversation")?;
        client
            .execute(
                "UPDATE images SET bytes = NULL WHERE conversation_id = $1",
                &[&id],
            )
            .context("postgres prune images")?;
        Ok(n > 0)
    }

    fn purge(&self, id: &str) -> Result<bool> {
        let mut client = lock(&self.client)?;
        client
            .execute("DELETE FROM images WHERE conversation_id = $1", &[&id])
            .context("postgres purge images")?;
        let n = client
            .execute("DELETE FROM conversations WHERE id = $1", &[&id])
            .context("postgres purge conversation")?;
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
        let byte_len = byte_len as i64;
        let created = now_secs() as i64;
        let bytes = bytes.to_vec();
        let caption = caption.map(str::to_string);
        let source = source.map(str::to_string);
        let mut client = lock(&self.client)?;
        let seq: i32 = client
            .query_one(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM images WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .context("postgres next image seq")?
            .get(0);
        client
            .execute(
                "INSERT INTO images (conversation_id, seq, caption, mime, bytes, byte_len, source, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &conversation_id,
                    &seq,
                    &caption,
                    &mime,
                    &bytes,
                    &byte_len,
                    &source,
                    &created,
                ],
            )
            .context("postgres insert image")?;
        Ok(ImageMeta {
            conversation_id: conversation_id.to_string(),
            seq: seq as i64,
            caption,
            mime: mime.to_string(),
            byte_len: byte_len as u64,
            has_bytes: true,
            source,
        })
    }

    fn load_image(&self, conversation_id: &str, seq: i64) -> Result<Option<ImageBlob>> {
        let seq_i32 = seq as i32;
        let mut client = lock(&self.client)?;
        let row = client
            .query_opt(
                "SELECT caption, mime, bytes, byte_len, source
                 FROM images WHERE conversation_id = $1 AND seq = $2",
                &[&conversation_id, &seq_i32],
            )
            .context("postgres load_image")?;
        match row {
            None => Ok(None),
            Some(row) => {
                let bytes: Option<Vec<u8>> = row.get(2);
                let has_bytes = bytes.as_ref().map(|b| !b.is_empty()).unwrap_or(false);
                Ok(Some(ImageBlob {
                    meta: ImageMeta {
                        conversation_id: conversation_id.to_string(),
                        seq,
                        caption: row.get(0),
                        mime: row.get(1),
                        byte_len: row.get::<_, i64>(3) as u64,
                        has_bytes,
                        source: row.get(4),
                    },
                    bytes: bytes.unwrap_or_default(),
                }))
            }
        }
    }

    fn list_images(&self, conversation_id: &str) -> Result<Vec<ImageMeta>> {
        let mut client = lock(&self.client)?;
        let rows = client
            .query(
                "SELECT seq, caption, mime,
                        bytes IS NOT NULL AND octet_length(bytes) > 0,
                        byte_len, source
                 FROM images WHERE conversation_id = $1 ORDER BY seq",
                &[&conversation_id],
            )
            .context("postgres list_images")?;
        Ok(rows
            .iter()
            .map(|row| ImageMeta {
                conversation_id: conversation_id.to_string(),
                seq: row.get::<_, i32>(0) as i64,
                caption: row.get(1),
                mime: row.get(2),
                has_bytes: row.get(3),
                byte_len: row.get::<_, i64>(4) as u64,
                source: row.get(5),
            })
            .collect())
    }

    fn is_empty(&self) -> Result<bool> {
        let mut client = lock(&self.client)?;
        let n: i64 = client
            .query_one("SELECT COUNT(*) FROM conversations", &[])?
            .get(0);
        Ok(n == 0)
    }
}

fn lock(client: &Mutex<Client>) -> Result<std::sync::MutexGuard<'_, Client>> {
    client
        .lock()
        .map_err(|_| anyhow::anyhow!("postgres connection lock poisoned"))
}

fn row_to_conv(row: &Row) -> Result<Conversation> {
    let chunks: String = row.get(6);
    Ok(Conversation {
        id: row.get(0),
        parent_id: row.get(1),
        title: row.get(2),
        created_at: row.get::<_, i64>(3) as u64,
        latest_message: row.get(4),
        brief: row.get(5),
        chunks: decode_chunks(&chunks)?,
        summary: row.get(7),
        status: Status::parse(&row.get::<_, String>(8)),
        updated_at: row.get::<_, i64>(9) as u64,
        last_saved_at: row.get::<_, i64>(10) as u64,
        pruned_at: row.get::<_, Option<i64>>(11).map(|v| v as u64),
        chunk_count: row.get::<_, i64>(12) as usize,
        images: Vec::new(),
    })
}

fn connect(cfg: &StoreConfig) -> Result<Client> {
    let target = parse_pg_target(&cfg.url)?;
    let user = if cfg.user.trim().is_empty() {
        target.user.unwrap_or_default()
    } else {
        cfg.user.clone()
    };
    let password = if cfg.password.is_empty() {
        target.password.unwrap_or_default()
    } else {
        cfg.password.clone()
    };
    if user.is_empty() {
        anyhow::bail!("postgres user is required (config store.user or in the url)");
    }

    let mut pg = postgres::Config::new();
    pg.host(&target.host);
    pg.port(target.port);
    pg.dbname(&target.dbname);
    pg.user(&user);
    if !password.is_empty() {
        pg.password(&password);
    }

    let ssl = cfg.ssl;
    match ssl {
        Some(true) => connect_tls(&pg),
        Some(false) => pg.connect(NoTls).context("connect postgres"),
        None => connect_tls(&pg).or_else(|tls_err| {
            pg.connect(NoTls)
                .with_context(|| format!("connect postgres (tls failed: {tls_err:#})"))
        }),
    }
}

fn connect_tls(pg: &postgres::Config) -> Result<Client> {
    let connector = native_tls::TlsConnector::builder()
        .build()
        .context("build tls connector")?;
    let connector = postgres_native_tls::MakeTlsConnector::new(connector);
    pg.connect(connector).context("connect postgres (tls)")
}

struct PgTarget {
    host: String,
    port: u16,
    dbname: String,
    user: Option<String>,
    password: Option<String>,
}

fn parse_pg_target(url: &str) -> Result<PgTarget> {
    let url = url.trim();
    if url.is_empty() {
        anyhow::bail!("postgres url is required (host:port/database)");
    }

    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .unwrap_or(url);

    let (auth_host, dbname) = rest
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("postgres url must look like host:port/database"))?;
    let dbname = dbname
        .split('?')
        .next()
        .unwrap_or(dbname)
        .trim()
        .to_string();
    if dbname.is_empty() {
        anyhow::bail!("postgres database name is missing");
    }

    let (user, password, hostport) = if let Some((auth, hostport)) = auth_host.rsplit_once('@') {
        if let Some((user, pass)) = auth.split_once(':') {
            (Some(user.to_string()), Some(pass.to_string()), hostport)
        } else {
            (Some(auth.to_string()), None, hostport)
        }
    } else {
        (None, None, auth_host)
    };

    let (host, port) = if let Some((host, port)) = hostport.rsplit_once(':') {
        let port: u16 = port
            .parse()
            .with_context(|| format!("invalid postgres port {port}"))?;
        (host.to_string(), port)
    } else {
        (hostport.to_string(), 5432)
    };
    if host.is_empty() {
        anyhow::bail!("postgres host is missing");
    }

    Ok(PgTarget {
        host,
        port,
        dbname,
        user,
        password,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_and_url_forms() {
        let a = parse_pg_target("db.internal:5432/handoff").unwrap();
        assert_eq!(a.host, "db.internal");
        assert_eq!(a.port, 5432);
        assert_eq!(a.dbname, "handoff");

        let b = parse_pg_target("postgres://alice:s3cret@db.internal:6543/handoff").unwrap();
        assert_eq!(b.user.as_deref(), Some("alice"));
        assert_eq!(b.password.as_deref(), Some("s3cret"));
        assert_eq!(b.port, 6543);
    }
}
