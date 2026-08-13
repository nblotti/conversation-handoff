# conversation-handoff

Hand off a **full Claude Code or Codex conversation** to a new chat when the context window is full.

The new chat does not get the whole transcript. You paste a **one-line reference**. The new session calls `load` to pull the brief about the latest message, and `recall` if you ask about the past conversation. Lookups walk parent threads.

You do **not** need Rust to run it. Download a release binary (or use the install script), then register it with Claude and Codex.

## What it does

1. In a long session, the model can `remember` important facts before they fall out of the window.
2. When the window is full, it calls `handoff` with:
   - `thread_id` — this conversation
   - `new_conversation_id` — id for the next chat
   - `latest_message` — the latest user request
   - `context` — only the recent relevant notes (not the full history)
3. You paste **only** the one-line `reference` into the new chat:

   `conversation-handoff: <new-id>`
4. The new chat calls `load` with that id and works from the stored brief.
5. If you say to look in the past conversation, it calls `recall`. A specific question finds matching parts; a vague request returns extra parent context.
6. The new chat can `handoff` again. Threads nest.

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

The skill in this repo (`.claude/skills/conversation-handoff/`) tells Claude when to hand off, to paste only the one-line reference, and to `load` / `recall` in the next chat. After `install`, that skill is also in your user skills folder, so it works in every project.

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
| `handoff` | Link thread → new id, store context, return a one-line `reference` |
| `load` | Pull the stored brief for that id (call this in the new chat) |
| `recall` | Extra parent context, or the parts that match a question |
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

Default is JSON files on disk:

- Linux: `~/.local/share/conversation-handoff/`
- Windows: `%APPDATA%\conversation-handoff\`
- Override: `CONVERSATION_HANDOFF_HOME`

To use SQLite (local embedded DB, like H2) or PostgreSQL, write a YAML config:

```bash
conversation-handoff init-config
```

That creates `~/.config/conversation-handoff/config.yaml` (Linux) or `%APPDATA%\conversation-handoff\config.yaml` (Windows). Override the path with `CONVERSATION_HANDOFF_CONFIG`.

```yaml
store:
  type: postgres          # file | sqlite | postgres
  url: "db.example.com:5432/conversation_handoff"
  user: handoff
  password: "change-me"
  ssl: true               # optional; omit to try TLS then plain
```

`type: sqlite` stores an embedded database at `url` (or `handoff.db` in the data dir if `url` is empty). `type: h2` and `type: local` are aliases for sqlite.

For postgres, `url` can also be `postgres://user:pass@host:5432/dbname`. `user` / `password` in the file override the URL. The password can come from `CONVERSATION_HANDOFF_DB_PASSWORD` instead of the YAML file.

See `config.example.yaml` in the repo.
