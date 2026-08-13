use anyhow::{bail, Result};
use serde::Serialize;

use crate::chunk::chunk_text;
use crate::rank::rank;
use crate::store::{now_secs, Conversation, ImageMeta, ListFilter, Status, Store};

const BRIEF_BUDGET: usize = 3500;
const DEFAULT_RECALL: usize = 6;
const DEFAULT_RECALL_CHARS: usize = 4000;
const SUMMARY_CHARS: usize = 120;

#[derive(Debug, Serialize)]
pub struct SaveResult {
    pub conversation_id: String,
    pub stored_chunks: usize,
    pub total_chunks: usize,
    pub previous_saved_at: u64,
    pub saved_at: u64,
    pub since_last_save: String,
    pub summary: Option<String>,
    pub attached_images: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct HandoffResult {
    /// One-line token to paste into the new chat. Nothing else.
    pub reference: String,
    pub thread_id: String,
    pub new_conversation_id: String,
    pub stored_chunks: usize,
    pub attached_images: Vec<String>,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct LoadResult {
    pub conversation_id: String,
    pub thread_id: Option<String>,
    pub parent_chain: Vec<String>,
    pub latest_message: Option<String>,
    pub brief: Option<String>,
    pub summary: Option<String>,
    pub status: Status,
    pub last_saved_at: u64,
    pub images: Vec<ImageRef>,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct ImageRef {
    pub reference: String,
    pub caption: Option<String>,
    pub mime: String,
    pub byte_len: u64,
    pub has_bytes: bool,
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
    pub summary: Option<String>,
    pub status: Status,
    pub latest_message: Option<String>,
    pub chunk_count: usize,
    pub image_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ThreadResult {
    pub conversation_id: String,
    pub links: Vec<ThreadLink>,
}

#[derive(Debug, Serialize)]
pub struct ConversationCard {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub summary: String,
    pub status: Status,
    pub age: String,
    pub chunk_count: usize,
    pub image_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ListResult {
    pub conversations: Vec<ConversationCard>,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct ForgetResult {
    pub forgotten: Vec<String>,
    pub mode: String,
    pub hint: String,
}

#[derive(Debug, Serialize)]
pub struct AttachImageResult {
    pub reference: String,
    pub conversation_id: String,
    pub seq: i64,
    pub mime: String,
    pub byte_len: u64,
    pub caption: Option<String>,
}

pub struct LoadedImage {
    pub meta: ImageMeta,
    pub bytes: Vec<u8>,
}

/// Image to store with save / handoff. Path or base64 is required.
#[derive(Debug, Default, Clone)]
pub struct ImageInput {
    pub path: Option<String>,
    pub data_base64: Option<String>,
    pub caption: Option<String>,
    pub mime: Option<String>,
}

pub struct Engine {
    store: Store,
}

impl Engine {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn help(&self) -> crate::help::HelpResult {
        crate::help::build(
            crate::config::config_path().display().to_string(),
            self.store.owner().to_string(),
            self.store.has_encryption(),
        )
    }

    pub fn save(
        &self,
        conversation_id: &str,
        text: &str,
        title: Option<String>,
        summary: Option<String>,
        images: &[ImageInput],
    ) -> Result<SaveResult> {
        let id = require_id(conversation_id, "conversation_id")?;
        let mut conv = self.store.get_or_create(id, None)?;
        if conv.status == Status::Pruned {
            bail!("conversation {id} is pruned; only the summary remains. Start a new id to keep writing.");
        }
        let previous_saved_at = conv.last_saved_at;
        if conv.title.is_none() {
            conv.title = title;
        }
        let added = chunk_text(text);
        let stored_chunks = added.len();
        conv.chunks.extend(added);
        if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
            conv.summary = Some(first_sentence(&summary, SUMMARY_CHARS));
        } else if conv.summary.is_none() {
            conv.summary = Some(derive_summary(&conv));
        }
        let now = now_secs();
        conv.updated_at = now;
        conv.last_saved_at = now;
        conv.sync_chunk_count();
        let total_chunks = conv.chunk_count;
        let summary = conv.summary.clone();
        self.store.save(&conv)?;
        let attached_images = self.attach_many(id, images)?;
        Ok(SaveResult {
            conversation_id: id.to_string(),
            stored_chunks,
            total_chunks,
            previous_saved_at,
            saved_at: now,
            since_last_save: since_last_save(previous_saved_at, now),
            summary,
            attached_images,
        })
    }

    pub fn handoff(
        &self,
        thread_id: &str,
        new_conversation_id: &str,
        latest_message: &str,
        context: &str,
        title: Option<String>,
        summary: Option<String>,
        images: &[ImageInput],
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
        if parent.status != Status::Pruned {
            parent.latest_message = Some(latest_message.to_string());
            parent.chunks.extend(chunks.clone());
            if parent.summary.is_none() {
                parent.summary = Some(derive_summary(&parent));
            }
            let now = now_secs();
            parent.updated_at = now;
            parent.last_saved_at = now;
            parent.sync_chunk_count();
            self.store.save(&parent)?;
        }

        if let Some(existing) = self.store.load(new_id)? {
            if existing.parent_id.as_deref() != Some(thread_id) {
                bail!(
                    "conversation {new_id} already exists and is linked to {:?}",
                    existing.parent_id
                );
            }
        }

        let brief = build_brief(latest_message, &chunks);
        let now = now_secs();
        let mut child = Conversation::new(new_id, Some(thread_id.to_string()));
        child.title = title;
        child.created_at = now;
        child.updated_at = now;
        child.last_saved_at = now;
        child.latest_message = Some(latest_message.to_string());
        child.brief = Some(brief);
        child.summary = Some(
            summary
                .filter(|s| !s.trim().is_empty())
                .map(|s| first_sentence(&s, SUMMARY_CHARS))
                .unwrap_or_else(|| first_sentence(latest_message, SUMMARY_CHARS)),
        );
        self.store.save(&child)?;
        let image_target = if parent.status == Status::Pruned {
            new_id
        } else {
            thread_id
        };
        let attached_images = self.attach_many(image_target, images)?;

        Ok(HandoffResult {
            reference: reference_line(new_id),
            thread_id: thread_id.to_string(),
            new_conversation_id: new_id.to_string(),
            stored_chunks: chunks.len(),
            attached_images,
            hint: format!(
                "Show the user only this line: {}. In the new chat, call load with that id. Do not paste the brief.",
                reference_line(new_id)
            ),
        })
    }

    /// Pull the stored brief for a continuation chat. Accepts a raw id or a `conversation-handoff: id` line.
    pub fn load(&self, conversation_id: &str) -> Result<LoadResult> {
        let conversation_id = parse_id(conversation_id)?;
        let conv = self.store.load(conversation_id)?.ok_or_else(|| {
            anyhow::anyhow!("unknown conversation_id {conversation_id}. Call handoff first.")
        })?;
        let chain = self.store.chain(conversation_id)?;
        let parent_chain: Vec<String> = chain.iter().map(|c| c.id.clone()).collect();
        let mut images = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for c in &chain {
            for img in &c.images {
                if seen.insert(img.reference()) {
                    images.push(image_ref(img));
                }
            }
        }
        let hint = if conv.status == Status::Pruned {
            "This conversation was pruned. Only the summary remains. Call list to pick another, or recall from a child that still has this id as parent.".to_string()
        } else {
            "Work from this brief. If the user asks about the past conversation, or something is missing, call recall. last_saved_at is the last checkpoint; /handoff save should cover work since then.".to_string()
        };
        let summary = Some(
            conv.summary
                .clone()
                .unwrap_or_else(|| derive_summary(&conv)),
        );
        Ok(LoadResult {
            conversation_id: conversation_id.to_string(),
            thread_id: conv.parent_id.clone(),
            parent_chain,
            latest_message: conv.latest_message,
            brief: conv.brief,
            summary,
            status: conv.status,
            last_saved_at: conv.last_saved_at,
            images,
            hint,
        })
    }

    pub fn recall(
        &self,
        conversation_id: &str,
        query: Option<&str>,
        max_results: Option<u32>,
    ) -> Result<RecallResult> {
        let conversation_id = parse_id(conversation_id)?;
        let query = query.unwrap_or("").trim();
        if query.is_empty() || is_broad_past_query(query) {
            return self.browse_past(conversation_id, max_results);
        }

        let chain = self.store.chain(conversation_id)?;
        if chain.is_empty() {
            bail!("unknown conversation_id {conversation_id}. Call handoff or save first.");
        }
        let parent_chain: Vec<String> = chain.iter().map(|c| c.id.clone()).collect();

        let mut corpus: Vec<(String, String)> = Vec::new();
        for conv in &chain {
            push_corpus(&mut corpus, conv);
        }

        let texts: Vec<String> = corpus.iter().map(|(_, t)| t.clone()).collect();
        let ranked = rank(&texts, query);
        let limit = max_results.unwrap_or(DEFAULT_RECALL as u32).clamp(1, 20) as usize;

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
                "These are the best-matching parts in the thread chain {:?}. Call recall again with a narrower query to go deeper. Image captions are included; call get_image with the id#n reference to see a picture.",
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

    fn browse_past(&self, conversation_id: &str, max_results: Option<u32>) -> Result<RecallResult> {
        let chain = self.store.chain(conversation_id)?;
        if chain.is_empty() {
            bail!("unknown conversation_id {conversation_id}. Call handoff or save first.");
        }
        let parent_chain: Vec<String> = chain.iter().map(|c| c.id.clone()).collect();
        let limit = max_results.unwrap_or(DEFAULT_RECALL as u32).clamp(1, 20) as usize;

        let mut matches = Vec::new();
        let mut used = 0usize;
        for conv in chain.iter().skip(1) {
            let extras = pruned_stub(conv)
                .into_iter()
                .chain(conv.chunks.iter().rev().cloned())
                .chain(conv.images.iter().filter_map(|img| {
                    img.caption
                        .as_deref()
                        .map(|c| format!("Image {} ({})", img.reference(), c))
                }));
            for text in extras {
                if matches.len() >= limit || used >= DEFAULT_RECALL_CHARS {
                    break;
                }
                used += text.len();
                matches.push(RecallMatch {
                    conversation_id: conv.id.clone(),
                    score: 0.0,
                    text,
                });
            }
        }

        let hint = if matches.is_empty() {
            "No extra parent context stored. Try a more specific recall query.".to_string()
        } else {
            "Additional parts from parent conversations. Call recall again with a specific question to go deeper.".to_string()
        };

        Ok(RecallResult {
            conversation_id: conversation_id.to_string(),
            query: "(past conversation)".to_string(),
            parent_chain,
            matches,
            hint,
        })
    }

    pub fn thread(&self, conversation_id: &str) -> Result<ThreadResult> {
        let conversation_id = parse_id(conversation_id)?;
        let chain = self.store.chain(conversation_id)?;
        if chain.is_empty() {
            bail!("unknown conversation_id {conversation_id}");
        }
        let links = chain
            .into_iter()
            .map(|c| ThreadLink {
                image_count: c.images.len(),
                id: c.id,
                parent_id: c.parent_id,
                title: c.title,
                summary: c.summary,
                status: c.status,
                latest_message: c.latest_message,
                chunk_count: c.chunk_count,
            })
            .collect();
        Ok(ThreadResult {
            conversation_id: conversation_id.to_string(),
            links,
        })
    }

    pub fn list(
        &self,
        older_than: Option<&str>,
        limit: Option<u32>,
        contains: Option<&str>,
        thread: Option<&str>,
        include_pruned: bool,
    ) -> Result<ListResult> {
        let now = now_secs();
        let older_than_secs = match older_than {
            Some(s) if !s.trim().is_empty() => Some(parse_duration(s)?),
            _ => None,
        };
        let limit = limit.unwrap_or(50).clamp(1, 200);
        let mut convs = if let Some(thread) = thread.map(str::trim).filter(|s| !s.is_empty()) {
            let id = parse_id(thread)?;
            let chain = self.store.chain(id)?;
            if chain.is_empty() {
                bail!("unknown conversation_id {id}");
            }
            chain
                .into_iter()
                .filter(|c| include_pruned || c.status != Status::Pruned)
                .filter(|c| match older_than_secs {
                    Some(d) => c.updated_at < now.saturating_sub(d),
                    None => true,
                })
                .collect()
        } else {
            self.store.list(&ListFilter {
                older_than_secs,
                now_secs: now,
                limit,
                include_pruned,
                owner: String::new(),
            })?
        };

        if let Some(needle) = contains.map(str::trim).filter(|s| !s.is_empty()) {
            let needle = needle.to_ascii_lowercase();
            convs.retain(|c| card_haystack(c).contains(&needle));
        }
        convs.truncate(limit as usize);
        for conv in &mut convs {
            if conv.images.is_empty() {
                conv.images = self.store.list_images(&conv.id)?;
            }
        }

        let conversations: Vec<ConversationCard> = convs.iter().map(|c| to_card(c, now)).collect();
        let hint = if conversations.is_empty() {
            "No conversations match. Try a broader list, or include_pruned.".to_string()
        } else {
            "Pick by the summary sentence, then call load (or /handoff use <id>).".to_string()
        };
        Ok(ListResult {
            conversations,
            hint,
        })
    }

    pub fn forget(
        &self,
        conversation_id: Option<&str>,
        older_than: Option<&str>,
        purge: bool,
    ) -> Result<ForgetResult> {
        let mut ids = Vec::new();
        if let Some(id) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
            ids.push(parse_id(id)?.to_string());
        }
        if let Some(older) = older_than.map(str::trim).filter(|s| !s.is_empty()) {
            let listed = self.list(Some(older), Some(200), None, None, false)?;
            for card in listed.conversations {
                if !ids.contains(&card.id) {
                    ids.push(card.id);
                }
            }
        }
        if ids.is_empty() {
            bail!("pass conversation_id or older_than (for example 30d)");
        }

        let mut forgotten = Vec::new();
        for id in ids {
            let ok = if purge {
                self.store.purge(&id)?
            } else {
                // Ensure a summary exists before dropping content.
                if let Some(mut conv) = self.store.load(&id)? {
                    if conv.status != Status::Pruned && conv.summary.is_none() {
                        conv.summary = Some(derive_summary(&conv));
                        self.store.save(&conv)?;
                    }
                }
                self.store.prune(&id)?
            };
            if ok {
                forgotten.push(id);
            }
        }

        let mode = if purge { "purged" } else { "pruned" };
        let hint = if purge {
            "Rows removed. Child conversations that pointed here will no longer find this parent."
                .to_string()
        } else {
            "Content and image bytes deleted. Id, parent link, and a one-sentence summary remain so old references still resolve.".to_string()
        };
        Ok(ForgetResult {
            forgotten,
            mode: mode.to_string(),
            hint,
        })
    }

    fn attach_many(&self, conversation_id: &str, images: &[ImageInput]) -> Result<Vec<String>> {
        let mut refs = Vec::new();
        for img in images {
            let has_path = img.path.as_deref().is_some_and(|s| !s.trim().is_empty());
            let has_data = img
                .data_base64
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty());
            if !has_path && !has_data {
                continue;
            }
            let attached = self.attach_image(
                conversation_id,
                img.caption.as_deref(),
                img.path.as_deref(),
                img.data_base64.as_deref(),
                img.mime.as_deref(),
            )?;
            refs.push(attached.reference);
        }
        Ok(refs)
    }

    pub fn attach_image(
        &self,
        conversation_id: &str,
        caption: Option<&str>,
        path: Option<&str>,
        data_base64: Option<&str>,
        mime: Option<&str>,
    ) -> Result<AttachImageResult> {
        let id = parse_id(conversation_id)?;
        let mut conv = self.store.get_or_create(id, None)?;
        if conv.status == Status::Pruned {
            bail!("conversation {id} is pruned; cannot attach images");
        }
        let (bytes, source) = read_image_bytes(path, data_base64)?;
        let max = self.store.max_image_bytes();
        if bytes.len() as u64 > max {
            bail!(
                "image is {} bytes; max_image_bytes is {max}. Compress it or raise store.max_image_bytes.",
                bytes.len()
            );
        }
        let mime = sniff_mime(&bytes, mime)?;
        let meta = self.store.add_image(
            id,
            caption.filter(|s| !s.trim().is_empty()),
            &mime,
            &bytes,
            source.as_deref(),
        )?;
        conv.updated_at = now_secs();
        self.store.save(&conv)?;
        Ok(AttachImageResult {
            reference: meta.reference(),
            conversation_id: id.to_string(),
            seq: meta.seq,
            mime: meta.mime,
            byte_len: meta.byte_len,
            caption: meta.caption,
        })
    }

    pub fn image(&self, reference: &str) -> Result<LoadedImage> {
        let (id, seq) = parse_image_ref(reference)?;
        let blob = self
            .store
            .load_image(id, seq)?
            .ok_or_else(|| anyhow::anyhow!("unknown image {id}#{seq}"))?;
        if !blob.meta.has_bytes {
            bail!(
                "image {id}#{seq} was pruned; caption: {}",
                blob.meta.caption.as_deref().unwrap_or("(none)")
            );
        }
        Ok(LoadedImage {
            meta: blob.meta,
            bytes: blob.bytes,
        })
    }
}

fn push_corpus(corpus: &mut Vec<(String, String)>, conv: &Conversation) {
    if conv.status == Status::Pruned {
        if let Some(stub) = pruned_stub(conv) {
            corpus.push((conv.id.clone(), stub));
        }
        return;
    }
    for chunk in &conv.chunks {
        corpus.push((conv.id.clone(), chunk.clone()));
    }
    if let Some(brief) = &conv.brief {
        corpus.push((conv.id.clone(), format!("Continuation brief:\n{brief}")));
    }
    for img in &conv.images {
        if let Some(caption) = &img.caption {
            corpus.push((
                conv.id.clone(),
                format!("Image {} ({})", img.reference(), caption),
            ));
        }
    }
}

fn pruned_stub(conv: &Conversation) -> Option<String> {
    if conv.status != Status::Pruned {
        return None;
    }
    let when = conv
        .pruned_at
        .map(format_date)
        .unwrap_or_else(|| "unknown date".to_string());
    let summary = conv.summary.clone().unwrap_or_else(|| derive_summary(conv));
    Some(format!("[pruned {when}] {summary}"))
}

fn to_card(conv: &Conversation, now: u64) -> ConversationCard {
    ConversationCard {
        id: conv.id.clone(),
        parent_id: conv.parent_id.clone(),
        title: conv.title.clone(),
        summary: conv.summary.clone().unwrap_or_else(|| derive_summary(conv)),
        status: conv.status,
        age: relative_age(now.saturating_sub(conv.updated_at)),
        chunk_count: conv.chunk_count,
        image_count: conv.images.len(),
    }
}

fn card_haystack(conv: &Conversation) -> String {
    let mut s = String::new();
    s.push_str(&conv.id);
    s.push(' ');
    if let Some(t) = &conv.title {
        s.push_str(t);
        s.push(' ');
    }
    if let Some(t) = &conv.summary {
        s.push_str(t);
        s.push(' ');
    }
    if let Some(t) = &conv.latest_message {
        s.push_str(t);
    }
    s.to_ascii_lowercase()
}

fn image_ref(img: &ImageMeta) -> ImageRef {
    ImageRef {
        reference: img.reference(),
        caption: img.caption.clone(),
        mime: img.mime.clone(),
        byte_len: img.byte_len,
        has_bytes: img.has_bytes,
    }
}

fn derive_summary(conv: &Conversation) -> String {
    if let Some(s) = nonempty(conv.summary.as_deref()) {
        return first_sentence(s, SUMMARY_CHARS);
    }
    if let Some(s) = nonempty(conv.latest_message.as_deref()) {
        if s.chars().count() >= 24 {
            return first_sentence(s, SUMMARY_CHARS);
        }
    }
    if let Some(s) = nonempty(conv.title.as_deref()) {
        return first_sentence(s, SUMMARY_CHARS);
    }
    for chunk in &conv.chunks {
        let chunk = chunk.trim();
        if chunk.is_empty() || chunk.starts_with("Latest message:") {
            continue;
        }
        return first_sentence(chunk, SUMMARY_CHARS);
    }
    if let Some(s) = nonempty(conv.latest_message.as_deref()) {
        return first_sentence(s, SUMMARY_CHARS);
    }
    conv.id.clone()
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn first_sentence(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    let cut = text
        .find(['.', '!', '?'])
        .map(|i| i + 1)
        .unwrap_or(text.len());
    let sentence: String = text[..cut].chars().take(max_chars).collect();
    let sentence = sentence.trim();
    if sentence.chars().count() < text.chars().count() && sentence.chars().count() == max_chars {
        format!("{sentence}…")
    } else {
        sentence.to_string()
    }
}

fn since_last_save(previous: u64, now: u64) -> String {
    if previous == 0 {
        return "first save".to_string();
    }
    format!(
        "{} since last save",
        relative_age(now.saturating_sub(previous)).trim_end_matches(" ago")
    )
}

fn relative_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn format_date(secs: u64) -> String {
    // YYYY-MM-DD from unix seconds, UTC. Good enough for a stub label.
    const DAY: u64 = 86400;
    let days = secs / DAY;
    let mut y = 1970u64;
    let mut rem = days;
    loop {
        let len = if is_leap(y) { 366 } else { 365 };
        if rem < len {
            break;
        }
        rem -= len;
        y += 1;
    }
    let months = [
        31u64,
        if is_leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 1u64;
    for len in months {
        if rem < len {
            break;
        }
        rem -= len;
        m += 1;
    }
    format!("{y:04}-{m:02}-{:02}", rem + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn parse_duration(raw: &str) -> Result<u64> {
    let s = raw.trim().to_ascii_lowercase();
    let (num, mul) = if let Some(n) = s.strip_suffix('d') {
        (n, 86400u64)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('w') {
        (n, 86400 * 7)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else {
        (s.as_str(), 86400)
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration {raw:?}; use 30d, 24h, or 1w"))?;
    Ok(n.saturating_mul(mul))
}

fn read_image_bytes(
    path: Option<&str>,
    data_base64: Option<&str>,
) -> Result<(Vec<u8>, Option<String>)> {
    if let Some(path) = path.map(str::trim).filter(|s| !s.is_empty()) {
        let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("read image {path}: {e}"))?;
        return Ok((bytes, Some(path.to_string())));
    }
    if let Some(b64) = data_base64.map(str::trim).filter(|s| !s.is_empty()) {
        let b64 = b64
            .strip_prefix("data:")
            .and_then(|s| s.split_once(',').map(|(_, d)| d))
            .unwrap_or(b64);
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(b64.replace('\n', "")))
            .map_err(|_| anyhow::anyhow!("data_base64 is not valid base64"))?;
        return Ok((bytes, None));
    }
    bail!("pass path or data_base64");
}

fn sniff_mime(bytes: &[u8], hint: Option<&str>) -> Result<String> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok("image/png".into());
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Ok("image/jpeg".into());
    }
    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Ok("image/gif".into());
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Ok("image/webp".into());
    }
    if let Some(h) = hint {
        let h = h.trim().to_ascii_lowercase();
        return match h.as_str() {
            "image/png" | "png" => Ok("image/png".into()),
            "image/jpeg" | "image/jpg" | "jpeg" | "jpg" => Ok("image/jpeg".into()),
            "image/gif" | "gif" => Ok("image/gif".into()),
            "image/webp" | "webp" => Ok("image/webp".into()),
            _ => bail!("unsupported image type {h:?}; use png, jpeg, gif, or webp"),
        };
    }
    bail!("unsupported image type; use png, jpeg, gif, or webp")
}

fn parse_image_ref(raw: &str) -> Result<(&str, i64)> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("conversation-handoff:")
        .map(str::trim)
        .unwrap_or(raw);
    let Some((id, seq)) = raw.rsplit_once('#') else {
        bail!("image reference must look like conversation-id#1");
    };
    let id = require_id(id, "conversation_id")?;
    let seq: i64 = seq
        .parse()
        .map_err(|_| anyhow::anyhow!("image sequence must be a number, got {seq:?}"))?;
    if seq < 1 {
        bail!("image sequence must be >= 1");
    }
    Ok((id, seq))
}

fn require_id<'a>(id: &'a str, name: &str) -> Result<&'a str> {
    let id = id.trim();
    if id.is_empty() {
        bail!("{name} is required");
    }
    Ok(id)
}

fn parse_id(raw: &str) -> Result<&str> {
    let raw = raw.trim();
    let id = raw
        .strip_prefix("conversation-handoff:")
        .map(str::trim)
        .unwrap_or(raw);
    require_id(id, "conversation_id")
}

fn reference_line(id: &str) -> String {
    format!("conversation-handoff: {id}")
}

fn is_broad_past_query(query: &str) -> bool {
    let q = query.to_lowercase();
    q.contains("past conversation")
        || q.contains("previous conversation")
        || q.contains("previous chat")
        || q.contains("previous thread")
        || q.contains("look in the past")
        || q.contains("look in past")
        || matches!(
            q.trim(),
            "past" | "history" | "earlier" | "before" | "parent" | "the past"
        )
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
        out.push('\n');
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
            out.push('\n');
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn engine() -> (tempfile::TempDir, Engine) {
        let dir = tempdir().unwrap();
        let store = Store::open_sqlite(dir.path().join("t.db")).unwrap();
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
                None,
                &[],
            )
            .unwrap();

        assert_eq!(result.thread_id, "t1");
        assert_eq!(result.new_conversation_id, "c2");
        assert_eq!(result.reference, "conversation-handoff: c2");
        assert!(!result.reference.contains("JWT_SECRET"));

        let loaded = engine.load("conversation-handoff: c2").unwrap();
        assert_eq!(loaded.thread_id.as_deref(), Some("t1"));
        let brief = loaded.brief.expect("stored brief");
        assert!(brief.contains("JWT_SECRET"));
        assert!(brief.contains("Latest request"));
        assert!(loaded.last_saved_at > 0);
    }

    #[test]
    fn recall_without_query_returns_parent_context() {
        let (_dir, engine) = engine();
        engine
            .handoff(
                "root",
                "child",
                "continue",
                "Decision: treat nested comments as whitespace.",
                None,
                None,
                &[],
            )
            .unwrap();
        let recalled = engine
            .recall("child", Some("look in past conversation"), None)
            .unwrap();
        assert!(recalled
            .matches
            .iter()
            .any(|m| m.text.to_lowercase().contains("whitespace")));
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
                None,
                &[],
            )
            .unwrap();
        engine
            .handoff(
                "mid",
                "leaf",
                "write the tests for nested comments",
                "Currently writing unit tests.",
                None,
                None,
                &[],
            )
            .unwrap();

        let recalled = engine
            .recall("leaf", Some("nested comments whitespace decision"), None)
            .unwrap();
        assert_eq!(recalled.parent_chain, vec!["leaf", "mid", "root"]);
        assert!(!recalled.matches.is_empty());
        assert!(recalled
            .matches
            .iter()
            .any(|m| m.text.to_lowercase().contains("whitespace")));
    }

    #[test]
    fn save_then_recall() {
        let (_dir, engine) = engine();
        let first = engine
            .save(
                "solo",
                "API base url is https://example.test/v2",
                None,
                None,
                &[],
            )
            .unwrap();
        assert_eq!(first.since_last_save, "first save");
        let second = engine
            .save(
                "solo",
                "timeout is 30s",
                None,
                Some("api client".into()),
                &[],
            )
            .unwrap();
        assert!(second.since_last_save.contains("since last save"));
        let recalled = engine.recall("solo", Some("api base url"), None).unwrap();
        assert!(recalled
            .matches
            .iter()
            .any(|m| m.text.contains("example.test")));
    }

    #[test]
    fn recall_through_pruned_parent_keeps_summary() {
        let (_dir, engine) = engine();
        engine
            .handoff(
                "root",
                "child",
                "continue",
                "Decision: treat nested comments as whitespace.",
                None,
                Some("parser whitespace decision".into()),
                &[],
            )
            .unwrap();
        engine.forget(Some("root"), None, false).unwrap();
        let recalled = engine
            .recall("child", Some("whitespace decision"), None)
            .unwrap();
        assert!(recalled
            .matches
            .iter()
            .any(|m| m.text.contains("pruned") && m.text.contains("whitespace")));
        let loaded = engine.load("root").unwrap();
        assert_eq!(loaded.status, Status::Pruned);
        assert!(loaded.brief.is_none());
        assert!(loaded
            .summary
            .as_deref()
            .unwrap()
            .to_lowercase()
            .contains("whitespace"));
    }

    #[test]
    fn list_shows_summary_sentence() {
        let (_dir, engine) = engine();
        engine
            .save(
                "a",
                "we switched the auth crate to jsonwebtoken",
                None,
                None,
                &[],
            )
            .unwrap();
        let listed = engine.list(None, None, Some("auth"), None, false).unwrap();
        assert_eq!(listed.conversations.len(), 1);
        assert!(listed.conversations[0]
            .summary
            .to_lowercase()
            .contains("auth"));
    }

    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn attach_and_recall_image_caption() {
        let (dir, engine) = engine();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, TINY_PNG).unwrap();
        engine.save("c1", "debugging CI", None, None, &[]).unwrap();
        let attached = engine
            .attach_image(
                "c1",
                Some("screenshot of the failing test"),
                Some(path.to_str().unwrap()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(attached.reference, "c1#1");
        let recalled = engine
            .recall("c1", Some("failing test screenshot"), None)
            .unwrap();
        assert!(recalled.matches.iter().any(|m| m.text.contains("c1#1")));
        let img = engine.image("c1#1").unwrap();
        assert_eq!(img.bytes, TINY_PNG);
    }

    #[test]
    fn save_and_handoff_store_images_by_default() {
        let (dir, engine) = engine();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, TINY_PNG).unwrap();
        let images = [ImageInput {
            path: Some(path.to_string_lossy().into_owned()),
            caption: Some("failing CI screenshot".into()),
            ..Default::default()
        }];
        let saved = engine
            .save("t1", "debugging CI", None, None, &images)
            .unwrap();
        assert_eq!(saved.attached_images, vec!["t1#1"]);

        let handed = engine
            .handoff(
                "t1",
                "c2",
                "continue from the screenshot",
                "See the CI screenshot.",
                None,
                None,
                &images,
            )
            .unwrap();
        assert_eq!(handed.attached_images, vec!["t1#2"]);

        let loaded = engine.load("c2").unwrap();
        let refs: Vec<_> = loaded.images.iter().map(|i| i.reference.as_str()).collect();
        assert!(refs.contains(&"t1#1"), "{refs:?}");
        assert!(refs.contains(&"t1#2"), "{refs:?}");
        assert_eq!(engine.image("t1#1").unwrap().bytes, TINY_PNG);
    }

    #[test]
    fn parse_duration_examples() {
        assert_eq!(parse_duration("30d").unwrap(), 30 * 86400);
        assert_eq!(parse_duration("24h").unwrap(), 24 * 3600);
        assert_eq!(parse_duration("1w").unwrap(), 7 * 86400);
        assert_eq!(parse_duration("7").unwrap(), 7 * 86400);
    }
}
