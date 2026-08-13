use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::{now_secs, Backend, Conversation};

/// Import v0.3/v0.4 JSON files into an empty SQLite database.
/// Progress goes to stderr so stdout stays free for MCP.
pub fn import_json_dir(backend: &impl Backend, dir: &Path) -> Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut imported = 0usize;
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let mut conv: Conversation =
            serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
        if conv.id.trim().is_empty() {
            continue;
        }
        if conv.updated_at == 0 {
            conv.updated_at = conv.created_at;
        }
        if conv.last_saved_at == 0 {
            conv.last_saved_at = conv.created_at;
        }
        if conv.chunk_count == 0 {
            conv.chunk_count = conv.chunks.len();
        }
        if conv.created_at == 0 {
            conv.created_at = now_secs();
        }
        backend.save(&conv)?;
        imported += 1;
    }
    Ok(imported)
}
