//! Flyway-style versioned SQL. Applied once at startup and recorded in
//! `schema_migrations`. Not the Flyway JVM tool — same idea, embedded in the binary.

use anyhow::{Context, Result};

pub struct Migration {
    pub version: i32,
    pub name: &'static str,
    pub sqlite: &'static str,
    pub postgres: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "conversations",
        sqlite: include_str!("migrations/V001__conversations.sqlite.sql"),
        postgres: include_str!("migrations/V001__conversations.postgres.sql"),
    },
    Migration {
        version: 2,
        name: "images_and_lifecycle",
        sqlite: include_str!("migrations/V002__images_and_lifecycle.sqlite.sql"),
        postgres: include_str!("migrations/V002__images_and_lifecycle.postgres.sql"),
    },
    Migration {
        version: 3,
        name: "owner",
        sqlite: include_str!("migrations/V003__owner.sqlite.sql"),
        postgres: include_str!("migrations/V003__owner.postgres.sql"),
    },
];

const SQLITE_BOOKKEEPING: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  applied_at INTEGER NOT NULL
);
"#;

const POSTGRES_BOOKKEEPING: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  applied_at BIGINT NOT NULL
);
"#;

pub fn apply_sqlite(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(SQLITE_BOOKKEEPING)
        .context("create schema_migrations")?;
    baseline_sqlite(conn)?;
    for m in MIGRATIONS {
        if is_applied_sqlite(conn, m.version)? {
            continue;
        }
        conn.execute_batch(m.sqlite)
            .with_context(|| format!("sqlite migration V{:03} {}", m.version, m.name))?;
        record_sqlite(conn, m)?;
    }
    Ok(())
}

pub fn apply_postgres(client: &mut postgres::Client) -> Result<()> {
    client
        .batch_execute(POSTGRES_BOOKKEEPING)
        .context("create schema_migrations")?;
    baseline_postgres(client)?;
    for m in MIGRATIONS {
        if is_applied_postgres(client, m.version)? {
            continue;
        }
        client
            .batch_execute(m.postgres)
            .with_context(|| format!("postgres migration V{:03} {}", m.version, m.name))?;
        record_postgres(client, m)?;
    }
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn is_applied_sqlite(conn: &rusqlite::Connection, version: i32) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
        rusqlite::params![version],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn record_sqlite(conn: &rusqlite::Connection, m: &Migration) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![m.version, m.name, now()],
    )?;
    Ok(())
}

fn baseline_sqlite(conn: &rusqlite::Connection) -> Result<()> {
    let applied: i64 = conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
        row.get(0)
    })?;
    if applied > 0 {
        return Ok(());
    }
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'conversations'",
        [],
        |row| row.get(0),
    )?;
    if exists > 0 {
        record_sqlite(conn, &MIGRATIONS[0])?;
    }
    Ok(())
}

fn is_applied_postgres(client: &mut postgres::Client, version: i32) -> Result<bool> {
    let n: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = $1",
            &[&version],
        )?
        .get(0);
    Ok(n > 0)
}

fn record_postgres(client: &mut postgres::Client, m: &Migration) -> Result<()> {
    client.execute(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
        &[&m.version, &m.name, &now()],
    )?;
    Ok(())
}

fn baseline_postgres(client: &mut postgres::Client) -> Result<()> {
    let applied: i64 = client
        .query_one("SELECT COUNT(*) FROM schema_migrations", &[])?
        .get(0);
    if applied > 0 {
        return Ok(());
    }
    let exists: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_schema = 'public' AND table_name = 'conversations'",
            &[],
        )?
        .get(0);
    if exists > 0 {
        record_postgres(client, &MIGRATIONS[0])?;
    }
    Ok(())
}
