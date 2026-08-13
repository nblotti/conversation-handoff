use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{Config, StoreKind};

mod file;
mod postgres;
mod sqlite;

const MAX_CHAIN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub created_at: u64,
    #[serde(default)]
    pub latest_message: Option<String>,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub chunks: Vec<String>,
}

trait Backend: Send + Sync {
    fn load(&self, id: &str) -> Result<Option<Conversation>>;
    fn save(&self, conv: &Conversation) -> Result<()>;
}

#[derive(Clone)]
pub struct Store {
    backend: std::sync::Arc<dyn Backend>,
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::from_config(&Config::load()?)
    }

    pub fn from_config(cfg: &Config) -> Result<Self> {
        match cfg.store.kind()? {
            StoreKind::File => {
                let dir = if cfg.store.url.trim().is_empty() {
                    default_dir()
                } else {
                    PathBuf::from(cfg.store.url.trim())
                };
                Self::open_at(dir)
            }
            StoreKind::Sqlite => {
                let path = if cfg.store.url.trim().is_empty() {
                    default_sqlite_path()
                } else {
                    PathBuf::from(cfg.store.url.trim())
                };
                Self::open_sqlite(&path)
            }
            StoreKind::Postgres => Self::open_postgres(&cfg.store),
        }
    }

    pub fn open_at(dir: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            backend: std::sync::Arc::new(file::FileBackend::open(dir.into())?),
        })
    }

    pub fn open_sqlite(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            backend: std::sync::Arc::new(sqlite::SqliteBackend::open(path.as_ref())?),
        })
    }

    pub fn open_postgres(cfg: &crate::config::StoreConfig) -> Result<Self> {
        Ok(Self {
            backend: std::sync::Arc::new(postgres::PostgresBackend::connect(cfg)?),
        })
    }

    pub fn load(&self, id: &str) -> Result<Option<Conversation>> {
        self.backend.load(id)
    }

    pub fn save(&self, conv: &Conversation) -> Result<()> {
        self.backend.save(conv)
    }

    pub fn get_or_create(&self, id: &str, parent_id: Option<String>) -> Result<Conversation> {
        if let Some(existing) = self.load(id)? {
            return Ok(existing);
        }
        let conv = Conversation {
            id: id.to_string(),
            parent_id,
            title: None,
            created_at: now_secs(),
            latest_message: None,
            brief: None,
            chunks: Vec::new(),
        };
        self.save(&conv)?;
        Ok(conv)
    }

    /// Walk `id` then each parent, oldest last. Detects cycles.
    pub fn chain(&self, id: &str) -> Result<Vec<Conversation>> {
        let mut out = Vec::new();
        let mut current = Some(id.to_string());
        let mut seen = std::collections::HashSet::new();
        while let Some(cid) = current {
            if !seen.insert(cid.clone()) {
                anyhow::bail!("conversation chain has a cycle at {cid}");
            }
            if out.len() >= MAX_CHAIN {
                anyhow::bail!("conversation chain is longer than {MAX_CHAIN} links");
            }
            match self.load(&cid)? {
                Some(conv) => {
                    current = conv.parent_id.clone();
                    out.push(conv);
                }
                None => {
                    if out.is_empty() {
                        return Ok(out);
                    }
                    break;
                }
            }
        }
        Ok(out)
    }
}

pub fn default_dir() -> PathBuf {
    data_root().join("conversations")
}

pub fn default_sqlite_path() -> PathBuf {
    data_root().join("handoff.db")
}

fn data_root() -> PathBuf {
    if let Ok(home) = std::env::var("CONVERSATION_HANDOFF_HOME") {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("conversation-handoff")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn encode_chunks(chunks: &[String]) -> Result<String> {
    serde_json::to_string(chunks).context("encode chunks")
}

pub(crate) fn decode_chunks(raw: &str) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(raw).context("decode chunks")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_tree(store: &Store) {
        store
            .save(&Conversation {
                id: "root".into(),
                parent_id: None,
                title: Some("root".into()),
                created_at: 1,
                latest_message: None,
                brief: None,
                chunks: vec!["old work".into()],
            })
            .unwrap();
        store
            .save(&Conversation {
                id: "child".into(),
                parent_id: Some("root".into()),
                title: None,
                created_at: 2,
                latest_message: Some("continue".into()),
                brief: Some("brief".into()),
                chunks: vec![],
            })
            .unwrap();
        store
            .save(&Conversation {
                id: "grandchild".into(),
                parent_id: Some("child".into()),
                title: None,
                created_at: 3,
                latest_message: None,
                brief: None,
                chunks: vec![],
            })
            .unwrap();
    }

    #[test]
    fn file_round_trip_and_parent_chain() {
        let dir = tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        sample_tree(&store);
        let ids: Vec<_> = store
            .chain("grandchild")
            .unwrap()
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(ids, vec!["grandchild", "child", "root"]);
    }

    #[test]
    fn sqlite_round_trip_and_parent_chain() {
        let dir = tempdir().unwrap();
        let store = Store::open_sqlite(dir.path().join("t.db")).unwrap();
        sample_tree(&store);
        let loaded = store.load("root").unwrap().unwrap();
        assert_eq!(loaded.chunks, vec!["old work"]);
        let ids: Vec<_> = store
            .chain("grandchild")
            .unwrap()
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(ids, vec!["grandchild", "child", "root"]);
    }

    #[test]
    fn detects_cycles() {
        let dir = tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        store
            .save(&Conversation {
                id: "a".into(),
                parent_id: Some("b".into()),
                title: None,
                created_at: 1,
                latest_message: None,
                brief: None,
                chunks: vec![],
            })
            .unwrap();
        store
            .save(&Conversation {
                id: "b".into(),
                parent_id: Some("a".into()),
                title: None,
                created_at: 1,
                latest_message: None,
                brief: None,
                chunks: vec![],
            })
            .unwrap();
        let err = store.chain("a").unwrap_err().to_string();
        assert!(err.contains("cycle"));
    }
}
