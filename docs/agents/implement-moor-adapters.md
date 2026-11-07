# Subagent 1 — Moor execution and three adapters

## Mission

Implement the **execution side** of the target architecture **inside the existing Moor Rust/Tauri fork**, not a new gateway or Docker service:

1. Agent-facing `moor_mcp_call_tool` with backward compatibility for existing `moor_call_tool`.
2. **API-to-MCP**: configured local/remote HTTP API operations exposed as real, searchable, profile-governed MCP tools.
3. **CLI-to-MCP**: configured, explicitly approved CLI operations exposed as MCP tools.
4. **MCP-to-MCP**: reuse Moor's existing downstream MCP client/session and `tools/call` routing; do not reimplement it.

Implement **one internal execution path** for all three adapters, using Moor's existing profile/risk-policy checks, argument validation, audit, timeout and error handling. The diagram label `execution_tool` means an **internal function/dispatcher**, not an extra public MCP tool.

## Before changing code

- Repo in Mac Local Coder: `/workspace/specdest-company/tools/moor`. Source-of-truth diagram: `my_local_structure.excalidraw`; architecture: `docs/my-local-structure.md`. Read `CONTEXT.md`, `docs/base-connector.md`, `docs/agent-profiles.md`, and `/workspace/specdest-company/tools/controller/agents.md`.
- Check `git status` and `git worktree list`. The main checkout already contains uncommitted architecture documentation and diagrams; **do not reset, stash, overwrite, or commit those unrelated changes**.
- Use an **independent task worktree/branch** (e.g. `feat/moor-adapters`) under the existing approved `/worktrees` location. Reuse an appropriate worktree if present. Do not work simultaneously with Agent 2 in the same checkout.
- Inspect these real implementation points before coding:
  - `src-tauri/src/sidecar/mcp/server.rs`: public `CALL_TOOL_NAME = "moor_call_tool"`, `tools/list`, `tools/call`, `call_and_map`.
  - `src-tauri/src/sidecar/services/server_manager.rs`: `call_tool_for_profile`, session routing, auditing, `tool_exposure_for_profile`.
  - `src-tauri/src/sidecar/services/tool_catalog.rs`, `tool_visibility.rs`, `tool_risk.rs`: existing shared catalog and allow/deny logic.
  - `src-tauri/src/sidecar/services/server_manager/session.rs` and `mcp/transport/mcp_client.rs`: existing stdio/HTTP MCP client.
  - `src-tauri/src/sidecar/db/` and `http/routes/servers.rs`: configuration and persistence.
- Do **not** change Base/Convex OAuth architecture, directly expose the Mac to the internet, or replace Moor's local `npx`/stdio installation and lifecycle support.

## Interfaces agreed with Agent 2 — do not break

Agent 2 owns the search implementation. Both sides use the **same policy-filtered tool catalog**. Every API/CLI/MCP operation newly registered by this agent must be discoverable through `tool_exposure_for_profile(...).callable_tools()` or the equivalent canonical catalog path, with these existing `ToolCatalogEntry` fields intact:

```
server_id, server_name, tool_name, exposed_name,
description, input_schema, risk_level
```

The search response contract remains:

```
{
  "tools": [{
    "name": "server__operation",
    "description": "...",
    "inputSchema": {"type": "object"},
    "serverName": "...",
    "riskLevel": "safe|write|destructive|unknown",
    "serverInstructions": "optional"
  }],
  "nextCursor": "optional"
}
```

`moor_search_tools` **only discovers** tools; `moor_mcp_call_tool` **executes** them. Do not create a new public `execution_tool`; do not rename/delete the existing `moor_search_tools` or alter `src-tauri/src/sidecar/services/tool_search.rs` (Agent 2 owns it). Keep server ID / exposed-name resolution stable and do not leak disabled or unassigned tools.

The currently implemented public name is `moor_call_tool`. The diagram requests `moor_mcp_call_tool`: **add compatibility** so clients using `moor_call_tool` continue to work; prefer exposing the requested name in curated mode while still accepting the legacy name. Both names must call the **same** permission-checked dispatcher, not duplicate executors. Update tests and descriptions accordingly.

## Implementation requirements

### A. Unified dispatcher / meta-tool

1. Preserve `tools/list` / `tools/call`, `MCP` JSON-RPC result/error shapes and direct pinned-tool calls.
2. Dispatch by a stable internal provider type (`mcp`, `api`, `cli`) **after** server/profile/tool permissions are resolved; no execution bypass through aliases, cached names, or direct calls.
3. Enforce registered JSON-schema arguments, tool availability, profile disabled-tools, write/destructive policy and timeout for every adapter. Keep one audit trail with tool, profile, origin, duration, sanitized args/result/error.
4. The agent-supplied target must be an **exact registered tool ID**, never an arbitrary command, URL, or MCP server address.

### B. MCP-to-MCP (existing)

- Keep and reuse `McpSession::list_tools/call_tool`, the stdio/remote HTTP connectors, credentials, lifecycle, and tool caching.
- Expose the same downstream tools through the unified dispatcher. Test at least one fake downstream stdio/HTTP server and an authorized/denied profile.
- No new intermediary network server or second MCP client stack.

### C. API-to-MCP

- Persist administrator-defined API operation definitions in Moor-owned config (not Base). Provide simple CRUD through existing Moor management/UI conventions where needed; reuse the existing DB/config pattern.
- Each operation has a stable ID, display name, description, input JSON schema, HTTP method, configured base URL/path, optional request mapping/headers, explicit risk classification, timeout, response size cap and credential **reference**. It appears in the same tool registry as MCP tools.
- The agent may supply **schema-valid parameter values**, not raw destination URL/host, arbitrary headers, API keys, unrestricted paths or shell commands. Avoid open-proxy/SSRF behavior; restrict permitted schemes and destinations (external HTTPS; explicitly configured loopback/container-local HTTP only), redirects and request sizes.
- Credentials remain in existing secure storage (prefer macOS Keychain) and are never returned in tool schemas, logs or search results.
- Test with a local mock HTTP API; include error/timeout, write permissions, invalid inputs, and forbidden destination tests.

### D. CLI-to-MCP

- Persist administrator-defined, stable **approved command definitions** in Moor-owned config. A registered tool selects a preapproved executable/script, fixed working directory and bounded input schema; safely construct arguments without shell interpolation.
- For project commands prefer existing project `nps local.*`, and for cross-project commands existing Devkit `specdest ...` **only where already permitted**. Use coding-container-reachable approved service interfaces, not unbounded native Mac process execution.
- **Mandatory governance:** Before adding, expanding, registering or exposing any host-executable command/allowlist/wrapper, obtain explicit user approval (per Controller `agents.md`). Do not auto-authorize `host_cli`, use `bash -c`, or add a generic `run_command` tool. If the existing command bridge cannot execute an approved action, finish the safe adapter and clearly report the approval/deployment blocker; do not invent a bypass.
- Explicitly bound cwd, environment, parameters, output, timeout, concurrency, and secret redaction. Test with a harmless mock CLI, not a real host command.

## Shared responsibilities and limits

- Moor (native Mac app) owns **all configurations**, per-server agent instructions, tool permissions and agent-to-profile mapping. Base/Convex only handles OAuth and the outbound job bridge.
- The coding container is a separate runtime. `localhost` in a Docker container is **not** the Mac host. No new bind mounts, tunnel changes, host gateway policy changes or secret sharing without approval.
- Keep per-MCP custom instructions advisory and visible in search/initialize; never treat them as authorization.
- No code edits in Agent 2's search/ranking modules. Agent 1 owns shared `mcp/server.rs` protocol dispatch changes. Communicate any required shared-catalog changes as an explicit interface contract for Agent 2.
- Treat the diagram as a **target**, not evidence that CLI/API adapters or OS sandboxing already exist. Do not claim Moor has no host privileges just because adapters are separate modules.

## Acceptance / tests

- Authenticated Base/Convex bridge and direct Moor profile paths still work; no changes to OAuth tokens, client mapping or connector pairing.
- A configured API tool, CLI tool and downstream MCP tool all appear in `moor_search_tools` (when allowed), execute through `moor_mcp_call_tool`, and return normal MCP results; existing `moor_call_tool` clients still work.
- Disabled tools and disallowed profiles cannot discover or execute operations, including by direct or legacy names; risky operations respect policies.
- Failed actions, validation errors, timeouts and secrets are handled without tool leaks or execution bypass.
- Add focused Rust unit/integration tests for config persistence, API/CLI adapters, MCP delegation, profile isolation, alias, audit and validation. Run repo Rust and frontend checks using available approved workflows. Do not weaken checks; report environment blockers factually.
- Keep work committed **only to the agent's own branch** (do not merge into `main`, deploy, push or modify a running app unless explicitly instructed). Report commit ID, verified tests, limitations, and any host-command approvals needed.
