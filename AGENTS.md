# Conversation handoff

This repo is an MCP server that Claude Code and Codex call when a chat is out of context.

When the window is nearly full: `handoff` with the current thread id, a new conversation id, the latest user message, and only recent relevant notes. Give the user `continuation_pack` to paste into the new chat.

In the new chat: work from the brief. Call `recall` with a specific question to fetch stored parent-thread parts. A continuation can itself be handed off.

During long sessions, call `remember` to checkpoint facts before they fall out of the window.

Do not dump a full transcript into `context`.
