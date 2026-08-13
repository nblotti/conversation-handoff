use rmcp::{
    handler::server::wrapper::Parameters,
    model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;

use crate::engine::Engine;
use crate::store::Store;

const INSTRUCTIONS: &str = "\
Use this server when a conversation is running out of context, when a new chat starts with a conversation-handoff reference, or when the user asks about a past conversation.

Workflow:
1. During a long session, call remember to checkpoint important facts.
2. When the window is nearly full, call handoff with thread_id, new_conversation_id, latest_message, and only recent relevant context.
3. Show the user ONLY the returned `reference` line (conversation-handoff: <id>). Do not paste a brief or transcript.
4. In a new chat that contains that reference, immediately call load with the id. Work from the returned brief.
5. If the user asks to look in the past conversation, or something is missing, call recall. A specific question finds matching parts; a vague 'past conversation' request returns extra parent context. Ask a narrower question to go deeper.
6. A continuation can itself be handed off. Chains can be arbitrarily long.

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
pub struct RememberParams {
    /// Conversation id to attach these notes to (usually the current session id).
    pub conversation_id: String,
    /// Facts, decisions, file paths, errors, or constraints to keep. Not a full transcript.
    pub text: String,
    /// Optional short title for this conversation.
    #[serde(default)]
    pub title: Option<String>,
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

#[tool_router]
impl ConversationService {
    #[tool(description = "Checkpoint important facts on a conversation before the context window fills. Call this during long work so a later handoff/recall has more than the last few messages.")]
    fn remember(&self, Parameters(p): Parameters<RememberParams>) -> String {
        match self.engine.remember(&p.conversation_id, &p.text, p.title) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(description = "Link a full conversation to a new one. Stores searchable context and returns a one-line `reference`. Show the user only that line. Do not paste a brief.")]
    fn handoff(&self, Parameters(p): Parameters<HandoffParams>) -> String {
        match self.engine.handoff(
            &p.thread_id,
            &p.new_conversation_id,
            &p.latest_message,
            &p.context,
            p.title,
        ) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(description = "Load the stored brief for a continuation chat. Call this immediately when the user (or first message) contains `conversation-handoff: <id>`.")]
    fn load(&self, Parameters(p): Parameters<LoadParams>) -> String {
        match self.engine.load(&p.conversation_id) {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(description = "Fetch more from parent conversations. Call this when the user asks to look in the past conversation, or when the brief is missing a fact. Walks parents recursively. A specific query finds matching parts; omit query for extra parent context.")]
    fn recall(&self, Parameters(p): Parameters<RecallParams>) -> String {
        match self
            .engine
            .recall(&p.conversation_id, p.query.as_deref(), p.max_results)
        {
            Ok(v) => json(&v),
            Err(e) => error(e),
        }
    }

    #[tool(description = "Show the linked conversation chain from this id back to the root thread.")]
    fn thread(&self, Parameters(p): Parameters<ThreadParams>) -> String {
        match self.engine.thread(&p.conversation_id) {
            Ok(v) => json(&v),
            Err(e) => error(e),
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
