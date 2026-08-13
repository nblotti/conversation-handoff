use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

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

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::open_at(default_dir())
    }

    pub fn open_at(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .with_context(|| format!("create data dir {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let conv: Conversation = serde_json::from_str(&data)
            .with_context(|| format!("parse {}", path.display()))?;
        Ok(Some(conv))
    }

    pub fn save(&self, conv: &Conversation) -> Result<()> {
        let path = self.path_for(&conv.id);
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_string_pretty(conv)?;
        fs::write(&tmp, data).with_context(|| format!("write {}", tmp.display()))?;
        replace_file(&tmp, &path)?;
        Ok(())
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
                bail!("conversation chain has a cycle at {cid}");
            }
            if out.len() >= MAX_CHAIN {
                bail!("conversation chain is longer than {MAX_CHAIN} links");
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

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(filename_for(id))
    }
}

pub fn default_dir() -> PathBuf {
    if let Ok(home) = std::env::var("CONVERSATION_HANDOFF_HOME") {
        return PathBuf::from(home).join("conversations");
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("conversation-handoff")
        .join("conversations")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn filename_for(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !safe.is_empty()
        && safe.len() <= 120
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        format!("{safe}.json")
    } else {
        format!("{}.json", to_hex(id.as_bytes()))
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn replace_file(tmp: &Path, dest: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        if dest.exists() {
            fs::remove_file(dest)
                .with_context(|| format!("remove {}", dest.display()))?;
        }
    }
    fs::rename(tmp, dest).with_context(|| format!("rename onto {}", dest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_and_parent_chain() {
        let dir = tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
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

        let chain = store.chain("grandchild").unwrap();
        let ids: Vec<_> = chain.iter().map(|c| c.id.as_str()).collect();
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
