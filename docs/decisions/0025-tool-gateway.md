# 0025 — The tool gateway: MCP over stdio through the core's socket

- **Status:** Accepted
- **Date:** 2026-11-19

## Context

[03](../spec/03-kernel.md#tool-gateway) says the kernel serves plugin-contributed tools to
agents over MCP, so a provider can hand its agent one MCP configuration. Open points: the
transport, how a session gets its endpoint, authentication, approvals and limits.

Every ACP agent must support stdio MCP servers; HTTP ones are optional. Claude Code takes
stdio servers in `--mcp-config`. A loopback TCP port can be reached by any local user, so
it would need its own authentication and would be a new listener.

## Decision

- **Transport:** MCP over stdio. A provider session whose plugin is granted `tools` gets
  an `mcp-server` record: the core binary's absolute path, `mcp --socket <socket>`, and
  `KUMIKIT_MCP_TOKEN` in the environment. The agent starts `kumikit mcp`, which connects
  to the core's local socket or pipe, sends `gateway.attach { token }` first, then copies
  MCP's newline-delimited JSON-RPC both ways. No port is opened; the socket's 0600 and
  peer-credential checks (the pipe's ACL on Windows) apply.
- **Tokens:** 32 random bytes per provider session, kept in memory, void when the session
  ends, the plugin is disabled or the core stops. `gateway.attach` is refused on remote
  connections.
- **Server:** Kumikit's own small MCP server: `initialize` (2025-03-26, 2025-06-18,
  2025-11-25), `ping`, `tools/list`, `tools/call`, cancellation and `list_changed`. No
  resources, prompts, sampling or elicitation.
- **Tools** are declared in the manifest (`[[contributes.tools]]`: name, title,
  description, input schema, `effect = "read" | "write"`) and served by the plugin's
  `kumikit:tool/tool.call` export. MCP names are `<plugin id with . and - as _>__<name>`.
- **Calls** are checked against the schema, run for the calling thread's workspace,
  logged as `tool.*` events with `source { plugin, tool }`, and refused outside a running
  turn of that thread.
- **Approvals:** `write` tools need an approval in the running turn; `allow_always` covers
  that tool in that thread. `read` tools run without asking.
- **Limits:** 1 MiB input and result, 4 calls in flight per session, the tool plugin's call
  limit and 5 minutes of wall time per call, cancelled with the turn.

## Consequences

- Any provider can pass the gateway on with a few lines; agents need nothing beyond stdio
  MCP.
- An agent that asks before MCP calls asks again for write tools.
- API providers that want HTTP endpoints, and remote agents, are not served (later).
