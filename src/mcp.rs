use rmcp::{
    handler::server::wrapper::Parameters,
    model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;

use crate::engine::Engine;
use crate::store::Store;

const INSTRUCTIONS: &str = "\
Use this server when a conversation is running out of context and work must continue in a new chat, \
or when a continuation chat needs older detail from its parent thread.

Workflow:
1. During a long session, call remember to checkpoint important facts before they fall out of the window.
2. When the window is nearly full, call handoff with:
   - thread_id: this conversation's id
   - new_conversation_id: a new unique id for the next chat
   - latest_message: the latest user request (the new chat continues from this)
   - context: only the recent relevant notes, decisions, files, and errors — not the full transcript
3. Give the user continuation_pack to paste as the first message of the new chat. Include the new conversation id.
4. In the new chat, work from the brief. If you need older detail, call recall with your conversation_id and a specific query. \
recall walks parent conversations recursively. Ask a narrower question to go deeper.
5. A continuation can itself be handed off. Chains can be arbitrarily long.

Never dump an entire transcript into handoff context. Store what matters for the latest request.";

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
pub struct RecallParams {
    /// The conversation you are in (or an ancestor you want to search from).
    pub conversation_id: String,
    /// Specific question or keywords. Narrower queries go deeper into the stored thread.
    pub query: String,
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

    #[tool(description = "Link a full conversation to a new one. Stores the thread, keeps searchable context, and returns a brief focused on the latest message (not the whole history). Give continuation_pack to the user to paste into the new chat.")]
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

    #[tool(description = "Fetch the stored parts most relevant to a question. Walks parent conversations recursively, so a continuation can reach a parent that itself has a parent. Call again with a narrower query to go deeper.")]
    fn recall(&self, Parameters(p): Parameters<RecallParams>) -> String {
        match self
            .engine
            .recall(&p.conversation_id, &p.query, p.max_results)
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
