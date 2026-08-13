use std::sync::Mutex;

use anyhow::{Context, Result};
use postgres::types::ToSql;
use postgres::{Client, NoTls, Row};

use crate::config::StoreConfig;

use super::{decode_chunks, encode_chunks, Backend, Conversation};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  parent_id TEXT,
  title TEXT,
  created_at BIGINT NOT NULL,
  latest_message TEXT,
  brief TEXT,
  chunks TEXT NOT NULL DEFAULT '[]'
);
"#;

pub struct PostgresBackend {
    client: Mutex<Client>,
}

impl PostgresBackend {
    pub fn connect(cfg: &StoreConfig) -> Result<Self> {
        let mut client = connect(cfg)?;
        client
            .batch_execute(SCHEMA)
            .context("create postgres schema")?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }
}

impl Backend for PostgresBackend {
    fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres connection lock poisoned"))?;
        let row = client
            .query_opt(
                "SELECT id, parent_id, title, created_at, latest_message, brief, chunks
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
        let mut client = self
            .client
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres connection lock poisoned"))?;
        let params: [&(dyn ToSql + Sync); 7] = [
            &conv.id,
            &conv.parent_id,
            &conv.title,
            &created,
            &conv.latest_message,
            &conv.brief,
            &chunks,
        ];
        client
            .execute(
                "INSERT INTO conversations (id, parent_id, title, created_at, latest_message, brief, chunks)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (id) DO UPDATE SET
                   parent_id = EXCLUDED.parent_id,
                   title = EXCLUDED.title,
                   created_at = EXCLUDED.created_at,
                   latest_message = EXCLUDED.latest_message,
                   brief = EXCLUDED.brief,
                   chunks = EXCLUDED.chunks",
                &params,
            )
            .context("postgres save")?;
        Ok(())
    }
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
