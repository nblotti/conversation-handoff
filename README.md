# conversation-handoff

Hand off a **full Claude Code or Codex conversation** to a new chat when the context window is full.

The new chat does not get the whole transcript. It gets a short brief about the **latest message**, plus a way to ask for older pieces later. Those lookups walk parent conversations, so a continuation can reach a thread that already has a parent.

You do **not** need Rust to run it. Download a release binary (or use the install script), then register it with Claude and Codex.

## What it does

1. In a long session, the model can `remember` important facts before they fall out of the window.
2. When the window is full, it calls `handoff` with:
   - `thread_id` — this conversation
   - `new_conversation_id` — id for the next chat
   - `latest_message` — the latest user request
   - `context` — only the recent relevant notes (not the full history)
3. You paste `continuation_pack` as the first message of the new chat.
4. If the new chat needs older detail, it calls `recall` with a specific question. The tool returns the stored parts that match. A narrower question goes deeper.
5. The new chat can `handoff` again. Threads nest.

## Install (no Rust)

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.sh | bash
```

Or download [conversation-handoff-linux-x86_64](https://github.com/nblotti/conversation-handoff/releases/latest) and put it on your `PATH`, then:

```bash
chmod +x conversation-handoff-linux-x86_64
mv conversation-handoff-linux-x86_64 ~/.local/bin/conversation-handoff
conversation-handoff install --write-instructions
```

`~/.local/bin` must be on your `PATH`.

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.ps1 | iex
```

Or download [conversation-handoff-windows-x86_64.exe](https://github.com/nblotti/conversation-handoff/releases/latest), then:

```powershell
mkdir $env:LOCALAPPDATA\conversation-handoff
move conversation-handoff-windows-x86_64.exe $env:LOCALAPPDATA\conversation-handoff\conversation-handoff.exe
& "$env:LOCALAPPDATA\conversation-handoff\conversation-handoff.exe" install --write-instructions
```

Add `%LOCALAPPDATA%\conversation-handoff` to your user PATH if the command is not found.

`install --write-instructions` registers the MCP server and copies the skill into:

- Claude: `~/.claude/skills/conversation-handoff/`
- Codex: `~/.agents/skills/conversation-handoff/`

Then **start a new Claude Code or Codex session** and approve the tools when asked.

## Configure Claude Code

The install command usually does this. To do it yourself:

```bash
claude mcp add --scope user --transport stdio conversation-handoff -- conversation-handoff
```

Or put this in `~/.claude.json` (all projects) or a project `.mcp.json`:

```json
{
  "mcpServers": {
    "conversation-handoff": {
      "type": "stdio",
      "command": "conversation-handoff"
    }
  }
}
```

On Windows, set `command` to the full path of `conversation-handoff.exe` if Claude cannot see it on `PATH`.

The skill in this repo (`.claude/skills/conversation-handoff/`) tells Claude when to hand off and how to call `recall`. After `install`, that skill is also in your user skills folder, so it works in every project.

## Configure Codex

```bash
codex mcp add conversation-handoff -- conversation-handoff
```

Or add this to `~/.codex/config.toml`:

```toml
[mcp_servers.conversation-handoff]
command = "conversation-handoff"
```

Use the absolute path to the `.exe` on Windows if needed.

The skill in `.agents/skills/conversation-handoff/` (and copied to `~/.agents/skills/` by install) tells Codex the same workflow. `AGENTS.md` in this repo is the project-level reminder.

## Tools

| Tool | What it does |
|------|----------------|
| `remember` | Store notes on a conversation id |
| `handoff` | Link thread → new id, store context, return a latest-message brief |
| `recall` | Return the stored parts that best match a query, walking parents |
| `thread` | Show the chain from this id back to the root |

## Build from source (optional)

Needs [Rust](https://rustup.rs/).

```bash
git clone https://github.com/nblotti/conversation-handoff
cd conversation-handoff
cargo install --path .
conversation-handoff install --write-instructions
```

## Data

Stored locally, one JSON file per conversation:

- Linux: `~/.local/share/conversation-handoff/`
- Windows: `%APPDATA%\conversation-handoff\`
- Override: `CONVERSATION_HANDOFF_HOME`
