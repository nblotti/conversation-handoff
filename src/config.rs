use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const SAMPLE: &str = r#"# conversation-handoff storage
#
# type:
#   sqlite    local embedded database (default)
#   postgres  PostgreSQL
#   file      accepted for older configs; mapped to sqlite and JSON files
#             in the data dir are imported once
#
# url:
#   sqlite    path to the .db file (empty = platform data dir / handoff.db)
#   postgres  host:port/database  or  postgres://host:port/database
#
# user / password: used for postgres. password may also come from
# CONVERSATION_HANDOFF_DB_PASSWORD.
#
# ssl: postgres only. omit to try TLS then plain. set false for no TLS.
#
# encryption_key: if set, title / latest_message / brief / chunks / summary
# and image bytes are encrypted at rest (ChaCha20-Poly1305). Can also come
# from CONVERSATION_HANDOFF_ENCRYPTION_KEY. Changing the key makes old rows
# unreadable.
#
# max_image_bytes: reject attached images larger than this (default 10485760).

store:
  type: sqlite
  url: ""
  user: ""
  password: ""
  encryption_key: ""
  max_image_bytes: 10485760
"#;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub store: StoreConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreConfig {
    /// file | sqlite | postgres
    #[serde(default = "default_store_type", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    /// Postgres TLS. Omit to try TLS then plain. Set false for no TLS.
    #[serde(default)]
    pub ssl: Option<bool>,
    /// If set, encrypt stored content. Also CONVERSATION_HANDOFF_ENCRYPTION_KEY.
    #[serde(default)]
    pub encryption_key: String,
    /// Reject attached images larger than this. 0 means the built-in 10 MB default.
    #[serde(default = "default_max_image_bytes")]
    pub max_image_bytes: u64,
}

fn default_store_type() -> String {
    "sqlite".into()
}

fn default_max_image_bytes() -> u64 {
    crate::store::DEFAULT_MAX_IMAGE_BYTES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    File,
    Sqlite,
    Postgres,
}

impl StoreConfig {
    pub fn kind(&self) -> Result<StoreKind> {
        match self.kind.trim().to_ascii_lowercase().as_str() {
            "" | "file" | "files" | "json" => Ok(StoreKind::File),
            "sqlite" | "local" | "h2" => Ok(StoreKind::Sqlite),
            "postgres" | "postgresql" | "pg" => Ok(StoreKind::Postgres),
            other => anyhow::bail!("unknown store.type {other:?}; use file, sqlite, or postgres"),
        }
    }

    pub fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("CONVERSATION_HANDOFF_STORE") {
            if !v.trim().is_empty() {
                self.kind = v;
            }
        }
        if let Ok(v) = std::env::var("CONVERSATION_HANDOFF_DB_URL") {
            if !v.trim().is_empty() {
                self.url = v;
            }
        }
        if let Ok(v) = std::env::var("CONVERSATION_HANDOFF_DB_USER") {
            if !v.trim().is_empty() {
                self.user = v;
            }
        }
        if let Ok(v) = std::env::var("CONVERSATION_HANDOFF_DB_PASSWORD") {
            if !v.is_empty() {
                self.password = v;
            }
        }
        if let Ok(v) = std::env::var("CONVERSATION_HANDOFF_ENCRYPTION_KEY") {
            if !v.is_empty() {
                self.encryption_key = v;
            }
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let mut cfg = if path.exists() {
            let text =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            parse_yaml(&text).with_context(|| format!("parse {}", path.display()))?
        } else {
            Config::default()
        };
        cfg.store.apply_env();
        Ok(cfg)
    }

    pub fn sample() -> &'static str {
        SAMPLE
    }
}

pub fn parse_yaml(text: &str) -> Result<Config> {
    let value: serde_yaml::Value = serde_yaml::from_str(text)?;
    if value.get("store").is_some() {
        Ok(serde_yaml::from_value(value)?)
    } else if value.get("type").is_some() {
        Ok(Config {
            store: serde_yaml::from_value(value)?,
        })
    } else {
        Ok(Config::default())
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("CONVERSATION_HANDOFF_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("conversation-handoff")
        .join("config.yaml")
}

pub fn write_sample(path: Option<PathBuf>) -> Result<PathBuf> {
    let path = path.unwrap_or_else(config_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, SAMPLE).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_and_flat_yaml() {
        let nested = parse_yaml(
            r#"
store:
  type: postgres
  url: "db:5432/handoff"
  user: alice
  password: secret
  ssl: false
"#,
        )
        .unwrap();
        assert_eq!(nested.store.kind().unwrap(), StoreKind::Postgres);
        assert_eq!(nested.store.user, "alice");
        assert_eq!(nested.store.ssl, Some(false));

        let flat = parse_yaml(
            r#"
type: sqlite
url: /tmp/handoff.db
"#,
        )
        .unwrap();
        assert_eq!(flat.store.kind().unwrap(), StoreKind::Sqlite);
        assert_eq!(flat.store.url, "/tmp/handoff.db");
    }

    #[test]
    fn aliases() {
        let h2 = parse_yaml("type: h2\n").unwrap();
        assert_eq!(h2.store.kind().unwrap(), StoreKind::Sqlite);
        let local = parse_yaml("type: local\n").unwrap();
        assert_eq!(local.store.kind().unwrap(), StoreKind::Sqlite);
    }
}
