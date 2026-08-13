---
name: conversation-handoff
description: Hand off a full Claude or Codex conversation to a new chat with a one-line reference, then load or recall stored context. Use when context is full, a message contains conversation-handoff:, the user asks to look in a past conversation, or older parent-thread detail is needed.
---

# Conversation handoff

Use the `conversation-handoff` MCP server. Never paste a long continuation pack.

## When the window is filling

1. Call `remember` with durable facts only.
2. Call `handoff` with `thread_id`, `new_conversation_id`, `latest_message`, and recent relevant `context`.
3. Show the user **only** the `reference` line, for example:

   `conversation-handoff: <new-id>`

## In the new chat

If the first message contains `conversation-handoff: <id>`, immediately call `load` with that id. Work from the returned brief.

## When the user asks about the past

If they say to look in the past conversation, previous thread, or that something is missing, call `recall` with the same id. Omit `query` (or pass their words) for extra parent context. Pass a specific question to find matching parts. Call again with a narrower question to go deeper.
