# Conversation handoff

If the `conversation-handoff` MCP server is not installed: do **not** install Rust or run Cargo. On Linux run `curl -fsSL https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.sh | bash`. On Windows (PowerShell) run `irm https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.ps1 | iex`. Linux binaries are static musl and do not need a GLIBC upgrade. If the binary cannot run, stop; do not compile from source.

When the window is nearly full: call `handoff`, then show the user **only** the one-line `reference` (`conversation-handoff: <id>`). Do not paste a brief.

In a new chat that contains that line: call `load` immediately and work from the brief.

If the user asks to look in the past conversation, call `recall`. A specific question finds matching parts.

User chat commands: `/handoff` (save since last checkpoint), `/handoff new`, `/handoff list`, `/handoff use <id>`, `/handoff rm <id>`, `/handoff img <path>`, `/handoff help`.

Do not dump a full transcript into `context`.
