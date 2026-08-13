# Conversation handoff

When the window is nearly full: call `handoff`, then show the user **only** the one-line `reference` (`conversation-handoff: <id>`). Do not paste a brief.

In a new chat that contains that line: call `load` immediately and work from the brief.

If the user asks to look in the past conversation, call `recall`. A specific question finds matching parts.

Do not dump a full transcript into `context`.
