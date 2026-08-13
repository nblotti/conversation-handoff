use base64::Engine as _;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, Implementation, ProtocolVersion, ServerCapabilities,
        ServerInfo,
    },
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;

use crate::engine::Engine;
use crate::store::Store;

const INSTRUCTIONS: &str = "\
Use this server when a conversation is running out of context, when a new chat starts with a conversation-handoff reference, when the user types /handoff, or when they ask about a past conversation.

Workflow:
1. During a long session, call save with work since last_saved_at (user command: /handoff or /handoff save).
2. When the window is nearly full, call handoff with thread_id, new_conversation_id, latest_message, and only recent relevant context. User command: /handoff new.
3. Show the user ONLY the returned `reference` line (conversation-handoff: <id>). Do not paste a brief or transcript.
4. In a new chat that contains that reference, immediately call load with the id. Work from the returned brief.
5. If the user asks to look in the past conversation, or something is missing, call recall. A specific question finds matching parts; a vague 'past conversation' request returns extra parent context.
6. /handoff list shows YOUR conversations as one-sentence summaries. /handoff use <id> loads one. /handoff rm <id> prunes content but keeps the summary. /handoff img <path> attaches a screenshot. /handoff help prints the command list and where store.owner / store.encryption_key go in config.yaml.
7. load and recall return image references like id#1, never the pixels. Call get_image with that reference when you need to see the picture.
8. A continuation can itself be handed off. Chains can be arbitrarily long.

Never dump an entire transcript into handoff context. Never ask the user to copy a long continuation pack.";

#[derive(Clone)]
pub struct ConversationService {
    engine: std::sync::Arc<Engine>,
}

impl ConversationService {
    pub fn new(store: Store) -> Self {
        Self {
            engine: std::sync::Arc::new(Engine::new(store)),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SaveParams {
    /// Conversation id to attach these notes to (usually the current session id).
    pub conversation_id: String,
    /// Facts, decisions, file paths, errors, or constraints since last_saved_at. Not a full transcript.
    pub text: String,
    /// Optional short title for this conversation.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional one-sentence summary shown in list.
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HandoffParams {
    /// Current conversation / thread id that is running out of context.
    pub thread_id: String,
    /// Id to use for the new continuation conversation.
    pub new_conversation_id: String,
    /// The latest user message that the new conversation must continue from.
    pub latest_message: String,
    /// Recent work context only: decisions, files, errors, constraints. Do not dump the full transcript.
    pub context: String,
    /// Optional short title for the new conversation.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional one-sentence summary shown in list.
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LoadParams {
    /// Continuation id, or the full `conversation-handoff: <id>` line from the new chat.
    pub conversation_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    /// The conversation you are in (or the `conversation-handoff: <id>` line).
    pub conversation_id: String,
    /// What to look up. Omit or say "past conversation" to pull extra parent context. A specific question finds matching parts.
    #[serde(default)]
    pub query: Option<String>,
    /// Maximum number of parts to return (default 6, max 20).
    #[serde(default)]
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ThreadParams {
    /// Conversation id whose parent chain you want to inspect.
    pub conversation_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Only conversations older than this, e.g. 30d, 24h, 1w.
    #[serde(default)]
    pub older_than: Option<String>,
    /// Maximum cards to return (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Filter by text in id, title, or summary.
    #[serde(default)]
    pub contains: Option<String>,
    /// Only this conversation and its parents.
    #[serde(default)]
    pub thread: Option<String>,
    /// Include pruned conversations (summary only).
    #[serde(default)]
    pub include_pruned: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ForgetParams {
    /// Conversation to prune or purge.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Prune every conversation older than this, e.g. 30d.
    #[serde(default)]
    pub older_than: Option<String>,
    /// If true, delete the row instead of keeping a summary.
    #[serde(default)]
    pub purge: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AttachImageParams {
    /// Conversation to attach the image to.
    pub conversation_id: String,
    /// One-line caption used later by recall.
    #[serde(default)]
    pub caption: Option<String>,
    /// Path to a png/jpeg/gif/webp file.
    #[serde(default)]
    pub path: Option<String>,
    /// Raw image as base64 (or a data: URL). Use this if there is no file path.
    #[serde(default)]
    pub data_base64: Option<String>,
    /// MIME type if sniffing from bytes fails.
    #[serde(default)]
    pub mime: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetImageParams {
    /// Image reference from load/recall, e.g. `abc#1`.
    pub reference: String,
}

#[tool_router]
impl ConversationService {
    #[tool(
        description = "Checkpoint work on a conversation since last_saved_at. Call this for /handoff or /handoff save. Returns when the previous save was."
    )]
    fn save(&self, Parameters(p): Parameters<SaveParams>) -> String {
        match self
            .engine
            .save(&p.conversation_id, &p.text, p.title, p.summary)
        {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Link a full conversation to a new one. Stores searchable context and returns a one-line `reference`. Show the user only that line. Do not paste a brief. User command: /handoff new."
    )]
    fn handoff(&self, Parameters(p): Parameters<HandoffParams>) -> String {
        match self.engine.handoff(
            &p.thread_id,
            &p.new_conversation_id,
            &p.latest_message,
            &p.context,
            p.title,
            p.summary,
        ) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Load the stored brief for a continuation chat. Call this immediately when the user (or first message) contains `conversation-handoff: <id>`, or for /handoff use <id>."
    )]
    fn load(&self, Parameters(p): Parameters<LoadParams>) -> String {
        match self.engine.load(&p.conversation_id) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Fetch more from parent conversations. Call this when the user asks to look in the past conversation, or when the brief is missing a fact. Walks parents recursively. A specific query finds matching parts; omit query for extra parent context."
    )]
    fn recall(&self, Parameters(p): Parameters<RecallParams>) -> String {
        match self
            .engine
            .recall(&p.conversation_id, p.query.as_deref(), p.max_results)
        {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Show the linked conversation chain from this id back to the root thread."
    )]
    fn thread(&self, Parameters(p): Parameters<ThreadParams>) -> String {
        match self.engine.thread(&p.conversation_id) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "List stored conversations as one-sentence summaries so the user can pick one. User command: /handoff list or /handoff list 30d."
    )]
    fn list(&self, Parameters(p): Parameters<ListParams>) -> String {
        match self.engine.list(
            p.older_than.as_deref(),
            p.limit,
            p.contains.as_deref(),
            p.thread.as_deref(),
            p.include_pruned.unwrap_or(false),
        ) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Show /handoff commands and where to put store.owner and store.encryption_key in config.yaml. Call this for /handoff help. Show the user the `text` field."
    )]
    fn help(&self) -> String {
        json(&self.engine.help())
    }

    #[tool(
        description = "Delete conversation content but keep a one-sentence summary so old references still resolve. User command: /handoff rm <id> or /handoff clean 30d. Set purge to remove the row entirely."
    )]
    fn forget(&self, Parameters(p): Parameters<ForgetParams>) -> String {
        match self.engine.forget(
            p.conversation_id.as_deref(),
            p.older_than.as_deref(),
            p.purge.unwrap_or(false),
        ) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Attach a png/jpeg/gif/webp to a conversation. Pass a file path or base64. Returns a short reference like id#1. User command: /handoff img <path>."
    )]
    fn attach_image(&self, Parameters(p): Parameters<AttachImageParams>) -> String {
        match self.engine.attach_image(
            &p.conversation_id,
            p.caption.as_deref(),
            p.path.as_deref(),
            p.data_base64.as_deref(),
            p.mime.as_deref(),
        ) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(
        description = "Fetch image pixels for a reference from load/recall (e.g. abc#1). Other tools only return captions."
    )]
    fn get_image(&self, Parameters(p): Parameters<GetImageParams>) -> CallToolResult {
        match self.engine.image(&p.reference) {
            Ok(img) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&img.bytes);
                let caption = img
                    .meta
                    .caption
                    .clone()
                    .unwrap_or_else(|| img.meta.reference());
                CallToolResult::success(vec![
                    ContentBlock::text(format!("{} ({})", img.meta.reference(), caption)),
                    ContentBlock::image(b64, img.meta.mime),
                ])
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("Error: {e}"))]),
        }
    }
}

#[tool_handler]
impl ServerHandler for ConversationService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(INSTRUCTIONS.to_string())
    }
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("serialize error: {e}"))
}

fn error(err: impl std::fmt::Display) -> String {
    format!("Error: {err}")
}
