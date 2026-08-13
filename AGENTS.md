# Conversation handoff

When the window is nearly full: call `handoff`, then show the user **only** the one-line `reference` (`conversation-handoff: <id>`). Do not paste a brief.

In a new chat that contains that line: call `load` immediately and work from the brief.

If the user asks to look in the past conversation, call `recall`. A specific question finds matching parts.

User chat commands: `/handoff` (save since last checkpoint), `/handoff new`, `/handoff list`, `/handoff use <id>`, `/handoff rm <id>`, `/handoff img <path>`.

Do not dump a full transcript into `context`.
