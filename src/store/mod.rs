use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{Config, StoreKind};
use crate::crypto::Crypto;

mod legacy;
mod migrate;
mod postgres;
mod sqlite;

const MAX_CHAIN: usize = 64;
pub const DEFAULT_MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Active,
    Pruned,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Pruned => "pruned",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "pruned" => Status::Pruned,
            _ => Status::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMeta {
    pub conversation_id: String,
    pub seq: i64,
    #[serde(default)]
    pub caption: Option<String>,
    pub mime: String,
    pub byte_len: u64,
    pub has_bytes: bool,
    #[serde(default)]
    pub source: Option<String>,
}

impl ImageMeta {
    pub fn reference(&self) -> String {
        format!("{}#{}", self.conversation_id, self.seq)
    }
}

#[derive(Debug, Clone)]
pub struct ImageBlob {
    pub meta: ImageMeta,
    pub bytes: Vec<u8>,
}

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
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub last_saved_at: u64,
    #[serde(default)]
    pub pruned_at: Option<u64>,
    #[serde(default)]
    pub chunk_count: usize,
    #[serde(default)]
    pub images: Vec<ImageMeta>,
}

impl Conversation {
    pub fn new(id: impl Into<String>, parent_id: Option<String>) -> Self {
        let now = now_secs();
        Self {
            id: id.into(),
            parent_id,
            title: None,
            created_at: now,
            latest_message: None,
            brief: None,
            chunks: Vec::new(),
            summary: None,
            status: Status::Active,
            updated_at: now,
            last_saved_at: 0,
            pruned_at: None,
            chunk_count: 0,
            images: Vec::new(),
        }
    }

    pub fn sync_chunk_count(&mut self) {
        if self.status != Status::Pruned {
            self.chunk_count = self.chunks.len();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListFilter {
    pub older_than_secs: Option<u64>,
    pub now_secs: u64,
    pub limit: u32,
    pub include_pruned: bool,
}

impl Default for ListFilter {
    fn default() -> Self {
        Self {
            older_than_secs: None,
            now_secs: now_secs(),
            limit: 50,
            include_pruned: false,
        }
    }
}

trait Backend: Send + Sync {
    fn load(&self, id: &str) -> Result<Option<Conversation>>;
    fn save(&self, conv: &Conversation) -> Result<()>;
    fn list(&self, filter: &ListFilter) -> Result<Vec<Conversation>>;
    fn prune(&self, id: &str, pruned_at: u64) -> Result<bool>;
    fn purge(&self, id: &str) -> Result<bool>;
    fn add_image(
        &self,
        conversation_id: &str,
        caption: Option<&str>,
        mime: &str,
        bytes: &[u8],
        byte_len: u64,
        source: Option<&str>,
    ) -> Result<ImageMeta>;
    fn load_image(&self, conversation_id: &str, seq: i64) -> Result<Option<ImageBlob>>;
    fn list_images(&self, conversation_id: &str) -> Result<Vec<ImageMeta>>;
    fn is_empty(&self) -> Result<bool>;
}

#[derive(Clone)]
pub struct Store {
    backend: std::sync::Arc<dyn Backend>,
    crypto: Option<Crypto>,
    max_image_bytes: u64,
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::from_config(&Config::load()?)
    }

    pub fn from_config(cfg: &Config) -> Result<Self> {
        let kind = cfg.store.kind()?;
        let store = match kind {
            StoreKind::File | StoreKind::Sqlite => {
                let custom_url = cfg.store.url.trim();
                let (path, import_dir) = match kind {
                    StoreKind::File => {
                        let json_dir = if custom_url.is_empty() {
                            default_dir()
                        } else {
                            PathBuf::from(custom_url)
                        };
                        (default_sqlite_path(), Some(json_dir))
                    }
                    StoreKind::Sqlite if custom_url.is_empty() => {
                        (default_sqlite_path(), Some(default_dir()))
                    }
                    StoreKind::Sqlite => (PathBuf::from(custom_url), None),
                    StoreKind::Postgres => unreachable!(),
                };
                Self::open_sqlite_maybe_import(path, import_dir)?
            }
            StoreKind::Postgres => Self::open_postgres(&cfg.store)?,
        };
        let store = store.with_max_image_bytes(cfg.store.max_image_bytes);
        store.with_encryption(cfg.store.encryption_key.trim())
    }

    pub fn with_max_image_bytes(mut self, max: u64) -> Self {
        self.max_image_bytes = if max == 0 {
            DEFAULT_MAX_IMAGE_BYTES
        } else {
            max
        };
        self
    }

    pub fn max_image_bytes(&self) -> u64 {
        self.max_image_bytes
    }

    pub fn with_encryption(mut self, key: &str) -> Result<Self> {
        if key.is_empty() {
            self.crypto = None;
            return Ok(self);
        }
        self.crypto = Some(Crypto::new(key)?);
        Ok(self)
    }

    pub fn open_sqlite(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_sqlite_maybe_import(path, None)
    }

    pub fn open_sqlite_maybe_import(
        path: impl AsRef<Path>,
        import_dir: Option<PathBuf>,
    ) -> Result<Self> {
        let backend = sqlite::SqliteBackend::open(path.as_ref())?;
        if let Some(dir) = import_dir {
            if backend.is_empty()? {
                match legacy::import_json_dir(&backend, &dir) {
                    Ok(0) => {}
                    Ok(n) => eprintln!(
                        "imported {n} conversations from {} into sqlite",
                        dir.display()
                    ),
                    Err(e) => eprintln!("legacy json import skipped: {e:#}"),
                }
            }
        }
        Ok(Self {
            backend: std::sync::Arc::new(backend),
            crypto: None,
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
        })
    }

    pub fn open_postgres(cfg: &crate::config::StoreConfig) -> Result<Self> {
        Ok(Self {
            backend: std::sync::Arc::new(postgres::PostgresBackend::connect(cfg)?),
            crypto: None,
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
        })
    }

    pub fn load(&self, id: &str) -> Result<Option<Conversation>> {
        let Some(mut conv) = self.backend.load(id)? else {
            return Ok(None);
        };
        if let Some(crypto) = &self.crypto {
            conv = crypto.decrypt_conv(conv)?;
        }
        conv.images = self.list_images(&conv.id)?;
        Ok(Some(conv))
    }

    pub fn save(&self, conv: &Conversation) -> Result<()> {
        let mut conv = conv.clone();
        conv.sync_chunk_count();
        match &self.crypto {
            Some(crypto) => self.backend.save(&crypto.encrypt_conv(&conv)?),
            None => self.backend.save(&conv),
        }
    }

    pub fn list(&self, filter: &ListFilter) -> Result<Vec<Conversation>> {
        let rows = self.backend.list(filter)?;
        let mut out = Vec::with_capacity(rows.len());
        for mut conv in rows {
            if let Some(crypto) = &self.crypto {
                conv = crypto.decrypt_conv(conv)?;
            }
            out.push(conv);
        }
        Ok(out)
    }

    pub fn prune(&self, id: &str) -> Result<bool> {
        self.backend.prune(id, now_secs())
    }

    pub fn purge(&self, id: &str) -> Result<bool> {
        self.backend.purge(id)
    }

    pub fn add_image(
        &self,
        conversation_id: &str,
        caption: Option<&str>,
        mime: &str,
        bytes: &[u8],
        source: Option<&str>,
    ) -> Result<ImageMeta> {
        let byte_len = bytes.len() as u64;
        let (stored, caption) = match &self.crypto {
            Some(crypto) => {
                let stored = crypto.encrypt_bytes(bytes)?;
                let caption = match caption {
                    Some(c) => Some(crypto.encrypt_text(c)?),
                    None => None,
                };
                (stored, caption)
            }
            None => (bytes.to_vec(), caption.map(str::to_string)),
        };
        let mut meta = self.backend.add_image(
            conversation_id,
            caption.as_deref(),
            mime,
            &stored,
            byte_len,
            source,
        )?;
        if let Some(crypto) = &self.crypto {
            meta.caption = match meta.caption {
                Some(c) => Some(crypto.decrypt_text(&c)?),
                None => None,
            };
        }
        Ok(meta)
    }

    pub fn load_image(&self, conversation_id: &str, seq: i64) -> Result<Option<ImageBlob>> {
        let Some(mut blob) = self.backend.load_image(conversation_id, seq)? else {
            return Ok(None);
        };
        if let Some(crypto) = &self.crypto {
            if blob.meta.has_bytes {
                blob.bytes = crypto.decrypt_bytes(&blob.bytes)?;
            }
            blob.meta.caption = match blob.meta.caption {
                Some(c) => Some(crypto.decrypt_text(&c)?),
                None => None,
            };
        }
        Ok(Some(blob))
    }

    pub fn list_images(&self, conversation_id: &str) -> Result<Vec<ImageMeta>> {
        let mut images = self.backend.list_images(conversation_id)?;
        if let Some(crypto) = &self.crypto {
            for img in &mut images {
                img.caption = match &img.caption {
                    Some(c) => Some(crypto.decrypt_text(c)?),
                    None => None,
                };
            }
        }
        Ok(images)
    }

    pub fn get_or_create(&self, id: &str, parent_id: Option<String>) -> Result<Conversation> {
        if let Some(existing) = self.load(id)? {
            return Ok(existing);
        }
        let conv = Conversation::new(id, parent_id);
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
        .unwrap_or_else(std::env::temp_dir)
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
        let mut root = Conversation::new("root", None);
        root.title = Some("root".into());
        root.created_at = 1;
        root.updated_at = 1;
        root.chunks = vec!["old work".into()];
        root.sync_chunk_count();
        store.save(&root).unwrap();

        let mut child = Conversation::new("child", Some("root".into()));
        child.created_at = 2;
        child.updated_at = 2;
        child.latest_message = Some("continue".into());
        child.brief = Some("brief".into());
        store.save(&child).unwrap();

        let mut grandchild = Conversation::new("grandchild", Some("child".into()));
        grandchild.created_at = 3;
        grandchild.updated_at = 3;
        store.save(&grandchild).unwrap();
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
        let store = Store::open_sqlite(dir.path().join("t.db")).unwrap();
        let mut a = Conversation::new("a", Some("b".into()));
        a.created_at = 1;
        store.save(&a).unwrap();
        let mut b = Conversation::new("b", Some("a".into()));
        b.created_at = 1;
        store.save(&b).unwrap();
        let err = store.chain("a").unwrap_err().to_string();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn sqlite_encrypts_content_at_rest() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        let store = Store::open_sqlite(&path)
            .unwrap()
            .with_encryption("unit-test-key")
            .unwrap();
        let mut conv = Conversation::new("c1", None);
        conv.title = Some("secret title".into());
        conv.created_at = 1;
        conv.latest_message = Some("do not leak".into());
        conv.brief = Some("brief secret".into());
        conv.summary = Some("summary secret".into());
        conv.chunks = vec!["chunk secret".into()];
        store.save(&conv).unwrap();

        let loaded = store.load("c1").unwrap().unwrap();
        assert_eq!(loaded.brief.as_deref(), Some("brief secret"));
        assert_eq!(loaded.summary.as_deref(), Some("summary secret"));
        assert_eq!(loaded.chunks, vec!["chunk secret"]);

        let conn = rusqlite::Connection::open(&path).unwrap();
        let brief: String = conn
            .query_row(
                "SELECT brief FROM conversations WHERE id = 'c1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(brief.starts_with("enc:v1:"), "{brief}");
        assert!(!brief.contains("brief secret"));

        let version: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(version >= 2);
    }

    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn sqlite_encrypts_image_bytes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("img.db");
        let store = Store::open_sqlite(&path)
            .unwrap()
            .with_encryption("unit-test-key")
            .unwrap();
        store.save(&Conversation::new("c1", None)).unwrap();
        let meta = store
            .add_image(
                "c1",
                Some("failing test"),
                "image/png",
                TINY_PNG,
                Some("shot.png"),
            )
            .unwrap();
        assert_eq!(meta.seq, 1);
        assert_eq!(meta.reference(), "c1#1");
        assert_eq!(meta.caption.as_deref(), Some("failing test"));
        assert_eq!(meta.byte_len, TINY_PNG.len() as u64);

        let loaded = store.load_image("c1", 1).unwrap().unwrap();
        assert_eq!(loaded.bytes, TINY_PNG);

        let conn = rusqlite::Connection::open(&path).unwrap();
        let raw: Vec<u8> = conn
            .query_row(
                "SELECT bytes FROM images WHERE conversation_id = 'c1' AND seq = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(&raw, TINY_PNG);
        assert!(!raw.windows(8).any(|w| w == &TINY_PNG[..8]));
    }

    #[test]
    fn prune_keeps_summary_drops_content() {
        let dir = tempdir().unwrap();
        let store = Store::open_sqlite(dir.path().join("t.db")).unwrap();
        let mut conv = Conversation::new("c1", None);
        conv.summary = Some("auth CI work".into());
        conv.brief = Some("long brief".into());
        conv.chunks = vec!["secret details".into()];
        store.save(&conv).unwrap();
        store
            .add_image("c1", Some("log"), "image/png", TINY_PNG, None)
            .unwrap();

        assert!(store.prune("c1").unwrap());
        let loaded = store.load("c1").unwrap().unwrap();
        assert_eq!(loaded.status, Status::Pruned);
        assert_eq!(loaded.summary.as_deref(), Some("auth CI work"));
        assert!(loaded.brief.is_none());
        assert!(loaded.chunks.is_empty());
        assert_eq!(loaded.chunk_count, 1);
        assert_eq!(loaded.images.len(), 1);
        assert!(!loaded.images[0].has_bytes);
        assert!(store.load_image("c1", 1).unwrap().unwrap().bytes.is_empty());
    }

    #[test]
    fn list_filters_by_age() {
        let dir = tempdir().unwrap();
        let store = Store::open_sqlite(dir.path().join("t.db")).unwrap();
        let mut old = Conversation::new("old", None);
        old.updated_at = 100;
        old.summary = Some("old work".into());
        store.save(&old).unwrap();
        let mut recent = Conversation::new("recent", None);
        recent.updated_at = 1_000_000;
        recent.summary = Some("recent work".into());
        store.save(&recent).unwrap();

        let listed = store
            .list(&ListFilter {
                older_than_secs: Some(50),
                now_secs: 200,
                limit: 50,
                include_pruned: false,
            })
            .unwrap();
        let ids: Vec<_> = listed.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["old"]);
    }

    #[test]
    fn legacy_json_import() {
        let dir = tempdir().unwrap();
        let json_dir = dir.path().join("conversations");
        std::fs::create_dir_all(&json_dir).unwrap();
        let conv = Conversation {
            id: "legacy1".into(),
            parent_id: None,
            title: Some("from json".into()),
            created_at: 42,
            latest_message: Some("hello".into()),
            brief: None,
            chunks: vec!["imported chunk".into()],
            summary: None,
            status: Status::Active,
            updated_at: 0,
            last_saved_at: 0,
            pruned_at: None,
            chunk_count: 0,
            images: vec![],
        };
        std::fs::write(
            json_dir.join("legacy1.json"),
            serde_json::to_string(&conv).unwrap(),
        )
        .unwrap();

        let store =
            Store::open_sqlite_maybe_import(dir.path().join("handoff.db"), Some(json_dir)).unwrap();
        let loaded = store.load("legacy1").unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("from json"));
        assert_eq!(loaded.chunks, vec!["imported chunk"]);
        assert_eq!(loaded.chunk_count, 1);
        assert_eq!(loaded.updated_at, 42);
    }
}
