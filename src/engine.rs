use anyhow::{bail, Result};
use serde::Serialize;

use crate::chunk::chunk_text;
use crate::rank::rank;
use crate::store::{now_secs, Store};

const BRIEF_BUDGET: usize = 3500;
const DEFAULT_RECALL: usize = 6;
const DEFAULT_RECALL_CHARS: usize = 4000;

#[derive(Debug, Serialize)]
pub struct HandoffResult {
    pub thread_id: String,
    pub new_conversation_id: String,
    pub parent_chain: Vec<String>,
    pub brief: String,
    pub stored_chunks: usize,
    pub continuation_pack: String,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct RecallMatch {
    pub conversation_id: String,
    pub score: f32,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct RecallResult {
    pub conversation_id: String,
    pub query: String,
    pub parent_chain: Vec<String>,
    pub matches: Vec<RecallMatch>,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct ThreadLink {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub latest_message: Option<String>,
    pub chunk_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ThreadResult {
    pub conversation_id: String,
    pub links: Vec<ThreadLink>,
}

#[derive(Debug, Serialize)]
pub struct RememberResult {
    pub conversation_id: String,
    pub stored_chunks: usize,
    pub total_chunks: usize,
}

pub struct Engine {
    store: Store,
}

impl Engine {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn remember(
        &self,
        conversation_id: &str,
        text: &str,
        title: Option<String>,
    ) -> Result<RememberResult> {
        let id = require_id(conversation_id, "conversation_id")?;
        let mut conv = self.store.get_or_create(id, None)?;
        if conv.title.is_none() {
            conv.title = title;
        }
        let added = chunk_text(text);
        let stored_chunks = added.len();
        conv.chunks.extend(added);
        let total_chunks = conv.chunks.len();
        self.store.save(&conv)?;
        Ok(RememberResult {
            conversation_id: id.to_string(),
            stored_chunks,
            total_chunks,
        })
    }

    pub fn handoff(
        &self,
        thread_id: &str,
        new_conversation_id: &str,
        latest_message: &str,
        context: &str,
        title: Option<String>,
    ) -> Result<HandoffResult> {
        let thread_id = require_id(thread_id, "thread_id")?;
        let new_id = require_id(new_conversation_id, "new_conversation_id")?;
        if thread_id == new_id {
            bail!("thread_id and new_conversation_id must be different");
        }
        let latest_message = latest_message.trim();
        if latest_message.is_empty() {
            bail!("latest_message is required — the new conversation continues from it");
        }

        let mut parent = self.store.get_or_create(thread_id, None)?;
        let mut chunks = chunk_text(context);
        if !latest_message.is_empty() {
            chunks.insert(0, format!("Latest message:\n{latest_message}"));
        }
        parent.latest_message = Some(latest_message.to_string());
        parent.chunks.extend(chunks.clone());
        self.store.save(&parent)?;

        if let Some(existing) = self.store.load(new_id)? {
            if existing.parent_id.as_deref() != Some(thread_id) {
                bail!(
                    "conversation {new_id} already exists and is linked to {:?}",
                    existing.parent_id
                );
            }
        }

        let brief = build_brief(latest_message, &chunks);
        let child = crate::store::Conversation {
            id: new_id.to_string(),
            parent_id: Some(thread_id.to_string()),
            title,
            created_at: now_secs(),
            latest_message: Some(latest_message.to_string()),
            brief: Some(brief.clone()),
            chunks: Vec::new(),
        };
        self.store.save(&child)?;

        let chain = self.store.chain(new_id)?;
        let parent_chain: Vec<String> = chain.iter().map(|c| c.id.clone()).collect();
        let continuation_pack = continuation_pack(new_id, thread_id, &brief, &parent_chain);

        Ok(HandoffResult {
            thread_id: thread_id.to_string(),
            new_conversation_id: new_id.to_string(),
            parent_chain,
            brief,
            stored_chunks: chunks.len(),
            continuation_pack,
            hint: format!(
                "Start a new conversation as {new_id}. Paste continuation_pack as the first message. Call recall with conversation_id={new_id} when you need older detail."
            ),
        })
    }

    pub fn recall(
        &self,
        conversation_id: &str,
        query: &str,
        max_results: Option<u32>,
    ) -> Result<RecallResult> {
        let conversation_id = require_id(conversation_id, "conversation_id")?;
        let query = query.trim();
        if query.is_empty() {
            bail!("query is required — ask for the specific fact you need");
        }

        let chain = self.store.chain(conversation_id)?;
        if chain.is_empty() {
            bail!("unknown conversation_id {conversation_id}. Call handoff or remember first.");
        }
        let parent_chain: Vec<String> = chain.iter().map(|c| c.id.clone()).collect();

        let mut corpus: Vec<(String, String)> = Vec::new();
        for conv in &chain {
            for chunk in &conv.chunks {
                corpus.push((conv.id.clone(), chunk.clone()));
            }
            if let Some(brief) = &conv.brief {
                corpus.push((conv.id.clone(), format!("Continuation brief:\n{brief}")));
            }
        }

        let texts: Vec<String> = corpus.iter().map(|(_, t)| t.clone()).collect();
        let ranked = rank(&texts, query);
        let limit = max_results
            .unwrap_or(DEFAULT_RECALL as u32)
            .clamp(1, 20) as usize;

        let mut matches = Vec::new();
        let mut used = 0usize;
        for (score, idx) in ranked {
            if matches.len() >= limit || used >= DEFAULT_RECALL_CHARS {
                break;
            }
            let (cid, text) = &corpus[idx];
            used += text.len();
            matches.push(RecallMatch {
                conversation_id: cid.clone(),
                score: (score * 1000.0).round() / 1000.0,
                text: text.clone(),
            });
        }

        let hint = if matches.is_empty() {
            "No matching parts. Try different keywords, or call thread to see the parent chain."
                .to_string()
        } else {
            format!(
                "These are the best-matching parts in the thread chain {:?}. Call recall again with a narrower query to go deeper.",
                parent_chain
            )
        };

        Ok(RecallResult {
            conversation_id: conversation_id.to_string(),
            query: query.to_string(),
            parent_chain,
            matches,
            hint,
        })
    }

    pub fn thread(&self, conversation_id: &str) -> Result<ThreadResult> {
        let conversation_id = require_id(conversation_id, "conversation_id")?;
        let chain = self.store.chain(conversation_id)?;
        if chain.is_empty() {
            bail!("unknown conversation_id {conversation_id}");
        }
        let links = chain
            .into_iter()
            .map(|c| ThreadLink {
                id: c.id,
                parent_id: c.parent_id,
                title: c.title,
                latest_message: c.latest_message,
                chunk_count: c.chunks.len(),
            })
            .collect();
        Ok(ThreadResult {
            conversation_id: conversation_id.to_string(),
            links,
        })
    }
}

fn require_id<'a>(id: &'a str, name: &str) -> Result<&'a str> {
    let id = id.trim();
    if id.is_empty() {
        bail!("{name} is required");
    }
    Ok(id)
}

fn build_brief(latest_message: &str, chunks: &[String]) -> String {
    let mut out = String::new();
    out.push_str("## Latest request\n");
    out.push_str(latest_message.trim());
    out.push_str("\n\n## Relevant context\n");

    let ranked = rank(chunks, latest_message);
    let mut used = out.len();
    let mut added = 0usize;

    for (_score, idx) in &ranked {
        let chunk = chunks[*idx].trim();
        if chunk.is_empty() {
            continue;
        }
        if chunk == latest_message || chunk.starts_with("Latest message:") {
            continue;
        }
        if used + chunk.len() + 4 > BRIEF_BUDGET {
            continue;
        }
        out.push_str("\n");
        out.push_str(chunk);
        out.push('\n');
        used += chunk.len() + 2;
        added += 1;
        if added >= 8 {
            break;
        }
    }

    if added == 0 {
        for chunk in chunks.iter().rev().take(3) {
            let chunk = chunk.trim();
            if chunk.starts_with("Latest message:") {
                continue;
            }
            if used + chunk.len() + 4 > BRIEF_BUDGET {
                continue;
            }
            out.push_str("\n");
            out.push_str(chunk);
            out.push('\n');
            used += chunk.len() + 2;
            added += 1;
        }
    }

    if added == 0 {
        out.push_str("\n(No extra context stored. Call recall if a parent thread has more.)\n");
    }

    out
}

fn continuation_pack(
    new_id: &str,
    thread_id: &str,
    brief: &str,
    chain: &[String],
) -> String {
    format!(
        "Continue work as conversation `{new_id}` in thread `{thread_id}`.\n\
         Parent chain (newest first): {}\n\n\
         {brief}\n\n\
         If you need older detail, call the conversation-handoff `recall` tool with conversation_id=`{new_id}` and a specific question. \
         That search walks parent conversations recursively. Ask a narrower question to go deeper.",
        chain.join(" -> ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn engine() -> (tempfile::TempDir, Engine) {
        let dir = tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        (dir, Engine::new(store))
    }

    #[test]
    fn handoff_brief_follows_latest_message() {
        let (_dir, engine) = engine();
        let result = engine
            .handoff(
                "t1",
                "c2",
                "the auth tests fail in CI because JWT_SECRET is unset",
                "Old: we migrated the billing tables last month.\n\n\
                 Auth: CI is missing JWT_SECRET and the login tests fail.\n\n\
                 Docs: README screenshots were updated yesterday.",
                Some("auth ci".into()),
            )
            .unwrap();

        assert_eq!(result.thread_id, "t1");
        assert_eq!(result.new_conversation_id, "c2");
        assert!(result.brief.contains("JWT_SECRET"));
        assert!(result.brief.contains("Latest request"));
        assert!(
            result.brief.to_lowercase().contains("auth")
                || result.brief.contains("JWT_SECRET")
        );
        assert!(result.parent_chain.contains(&"c2".to_string()));
        assert!(result.parent_chain.contains(&"t1".to_string()));
    }

    #[test]
    fn recall_walks_parent_chain() {
        let (_dir, engine) = engine();
        engine
            .handoff(
                "root",
                "mid",
                "keep going on the parser",
                "The parser rejects nested comments. Decision: treat them as whitespace.",
                None,
            )
            .unwrap();
        engine
            .handoff(
                "mid",
                "leaf",
                "write the tests for nested comments",
                "Currently writing unit tests.",
                None,
            )
            .unwrap();

        let recalled = engine
            .recall("leaf", "nested comments whitespace decision", None)
            .unwrap();
        assert_eq!(recalled.parent_chain, vec!["leaf", "mid", "root"]);
        assert!(!recalled.matches.is_empty());
        assert!(recalled
            .matches
            .iter()
            .any(|m| m.text.to_lowercase().contains("whitespace")));
    }

    #[test]
    fn remember_then_recall() {
        let (_dir, engine) = engine();
        engine
            .remember("solo", "API base url is https://example.test/v2", None)
            .unwrap();
        let recalled = engine.recall("solo", "api base url", None).unwrap();
        assert!(recalled.matches.iter().any(|m| m.text.contains("example.test")));
    }
}
