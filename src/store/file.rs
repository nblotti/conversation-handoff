use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Conversation, Backend};

pub struct FileBackend {
    dir: PathBuf,
}

impl FileBackend {
    pub fn open(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("create data dir {}", dir.display()))?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(filename_for(id))
    }
}

impl Backend for FileBackend {
    fn load(&self, id: &str) -> Result<Option<Conversation>> {
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

    fn save(&self, conv: &Conversation) -> Result<()> {
        let path = self.path_for(&conv.id);
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_string_pretty(conv)?;
        fs::write(&tmp, data).with_context(|| format!("write {}", tmp.display()))?;
        replace_file(&tmp, &path)?;
        Ok(())
    }
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
            fs::remove_file(dest).with_context(|| format!("remove {}", dest.display()))?;
        }
    }
    fs::rename(tmp, dest).with_context(|| format!("rename onto {}", dest.display()))?;
    Ok(())
}
