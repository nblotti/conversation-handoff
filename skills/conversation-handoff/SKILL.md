---
name: conversation-handoff
description: Hand off a full Claude or Codex conversation to a new chat when the context window is full, then recall only the stored parts that match a follow-up question. Use when context is full, the session must continue in a new conversation, a thread id and new conversation id are needed, or older parent-thread detail must be retrieved.
---

# Conversation handoff

Use the `conversation-handoff` MCP server. Do not dump a full transcript.

## When the window is filling

1. Call `remember` with the current conversation id and only durable facts (decisions, paths, errors, constraints).
2. When the window is nearly full, call `handoff`:
   - `thread_id`: this conversation
   - `new_conversation_id`: a new unique id
   - `latest_message`: the latest user request (the new chat continues from this)
   - `context`: recent relevant notes only
3. Give the user `continuation_pack` to paste as the first message of the new chat. Include the new conversation id.

## In the continuation chat

Work from the brief. If something is missing, call `recall` with your `conversation_id` and a **specific** question. Search walks parent conversations. A narrower question goes deeper. A continuation can `handoff` again.

Call `thread` to inspect the parent chain.
