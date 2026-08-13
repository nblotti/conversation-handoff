---
name: handoff
description: Hand off a full Claude or Codex conversation to a new chat with a one-line reference, then load, save, list, or recall stored context. Use when the user types /handoff, context is full, a message contains conversation-handoff:, they ask to look in a past conversation, or they want to list or delete old references.
---

# /handoff

Use the `conversation-handoff` MCP server. Never paste a long continuation pack.

## Install (if the MCP server is missing)

Do **not** install Rust or run Cargo. Linux releases are static musl binaries and run on AlmaLinux/RHEL without a GLIBC upgrade.

Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.sh | bash
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.ps1 | iex
```

Then start a new session and approve the tools. If the prebuilt binary cannot run, stop and report the error; do not compile from source.

## Commands

| User types | You call |
|------------|----------|
| `/handoff` or `/handoff save` | `save` with work **since** `last_saved_at` (from the last `load`/`save`). Include a one-sentence `summary`. |
| `/handoff new` | `handoff`. Show the user **only** the `reference` line. |
| `/handoff list` | `list` |
| `/handoff list 30d` | `list` with `older_than: 30d` |
| `/handoff use <id>` | `load` with that id |
| `/handoff rm <id>` | `forget` (keeps the summary, drops content and image bytes) |
| `/handoff clean 30d` | `forget` with `older_than: 30d` |
| `/handoff img <path>` | `attach_image` with a short caption |

## When the window is filling

1. Call `save` with durable facts since the last checkpoint.
2. Call `handoff` with `thread_id`, `new_conversation_id`, `latest_message`, and recent relevant `context`.
3. Show the user **only** the `reference` line, for example:

   `conversation-handoff: <new-id>`

## In the new chat

If the first message contains `conversation-handoff: <id>`, immediately call `load` with that id. Work from the returned brief. Note `last_saved_at` so the next `/handoff save` covers only new work.

## When the user asks about the past

If they say to look in the past conversation, previous thread, or that something is missing, call `recall` with the same id. Omit `query` (or pass their words) for extra parent context. Pass a specific question to find matching parts.

`load` / `recall` return image references like `id#1` plus captions. Call `get_image` with that reference only when you need the pixels.

## List and delete

`list` returns one sentence per conversation so the user can pick. After `forget`, the id still resolves: `load` returns the summary with `status: pruned`.
