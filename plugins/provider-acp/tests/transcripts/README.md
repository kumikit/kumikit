# Recorded ACP transcripts

Real output of an ACP agent, replayed through the adapter by [`../replay.rs`](../replay.rs)
and by the fake agent in end-to-end tests (`fake-acp-agent replay`). Each file is JSON
lines: `{"from": "client" | "agent", "message": …}`.

## Where they come from

Recorded on 2026-11-16 from Claude Code through `@agentclientprotocol/claude-agent-acp`
0.107.0, with `fake-acp-agent record <file> -- claude-agent-acp` between a small client
and the agent, in a temporary workspace holding one `README.md`:

| File | Prompt | User's answer |
|---|---|---|
| `claude-text.jsonl` | Reply with fixed words, no tools | — |
| `claude-tool-allow.jsonl` | Create `hello.txt` containing `hi` | Allow once |
| `claude-tool-deny.jsonl` | Run `touch denied.txt` | Reject |
| `claude-tool-readonly.jsonl` | Run `ls` (the agent runs it without asking) | — |
| `claude-cancel.jsonl` | Write a long essay; `session/cancel` after three updates | — |

The tool scenarios set the session mode to `default` (ask before changes).

## Scrubbing

Before committing, recordings are cleaned so nothing personal remains:

- `auth/status_update` and `_auth/status_update` notifications are dropped (they carry
  the account's email and plan), and so are `available_commands_update` notifications
  (they list the recording machine's installed skills).
- Every `_meta` object is removed (rate limits, quotas, internal ids).
- Session ids become `session-1`; the workspace path becomes `/workspace` (also where
  streamed text split it across chunks); home directories become `/home/user`.

Check with `grep -i -E '<your user>|<your email>|/tmp/|/home/<you>'` before committing a
new recording.
