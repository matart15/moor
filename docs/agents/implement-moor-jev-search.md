# Subagent 2 — Moor search and Jev-assisted tool ranking

## Mission

Implement the **discovery side** of Moor's target architecture:

1. Keep the existing agent-facing `moor_search_tools` (diagram label `search_tool`).
2. Improve relevant-tool discovery using **Jev-assisted search/ranking**, powered by the existing OpenRouter credential source if needed.
3. Return only **already permitted and registered** tools with exact schemas, metadata and per-MCP usage instructions. Discovery must never execute tools or grant permissions.

**Important terminology:** The inspected Controller code describes **JEV as a controlling executive runtime**, not a separately verified OpenRouter model or ranking API. Its `ai-thinking-mcp.mjs` calls OpenRouter to produce advisory options; this is **not** a tool-ranker interface. Before implementing, inspect whether a usable Jev inference/decision interface exists. If none exists, build a **Moor-local Jev-style advisory ranker** using OpenRouter; do not claim you called a non-existent Jev API. Ranking is advisory; deterministic Moor profile policy and registry remain authoritative.

## Before changing code

- Repo (Mac Local Coder): `/workspace/specdest-company/tools/moor`. Read `CONTEXT.md`, `docs/my-local-structure.md`, `docs/base-connector.md`, `docs/agent-profiles.md` and `/workspace/specdest-company/tools/controller/agents.md`.
- Check `git status` and `git worktree list`. Main has **uncommitted unrelated architecture diagrams/docs**. Work on an **independent task worktree/branch**, e.g. `feat/moor-jev-search`, under the existing approved `/worktrees` location; do not reset/stash/overwrite unrelated changes. Do not share an active checkout with Agent 1.
- Inspect the actual code before coding:
  - `src-tauri/src/sidecar/services/tool_search.rs` — currently implements `SEARCH_TOOL_NAME = "moor_search_tools"`, keyword ranking, pagination, scoped instructions and audit.
  - `src-tauri/src/sidecar/services/tool_catalog.rs` and `tool_visibility.rs` — canonical profile-visible catalog.
  - `src-tauri/src/sidecar/mcp/server.rs` — public search tool's RPC schema and `search_for_profile` call (Agent 1 owns edits here).
  - `src-tauri/src/sidecar/services/settings.rs`, `http/routes/settings.rs`, Moor Settings UI and existing keyring conventions for configuration.
  - `/workspace/specdest-company/tools/controller/scripts/internal/ai-thinking-mcp.mjs` — a current JEV-controlled advisory LLM implementation using OpenRouter (`https://openrouter.ai/api/v1/chat/completions`, `OPENROUTER_API_KEY`, configurable `AI_THINKING_MODEL`; inspect but **do not reuse its `think` response shape** for ranking).
  - `/workspace/specdest-company/tools/hermes-stack/README.md` and `scripts/check-openrouter.sh` — existing credential workflow. The ignored secret file **exists** at `/workspace/specdest-company/tools/hermes-stack/secrets/openrouter.env`. Do **not print/read its contents into logs, prompts, docs or source**, and do not assume the Docker path exists in the **native macOS Moor runtime**.

## Interfaces agreed with Agent 1 — do not break

- Agent 1 owns **execution adapters and shared `src-tauri/src/sidecar/mcp/server.rs`**. Do not edit that file, `server_manager.rs`, `tool_catalog.rs`, DB registry schema or the adapter implementations in your branch without an explicit integration agreement.
- You own `services/tool_search.rs`, its tests, and a new ranker module **nested beneath** it (e.g. `src-tauri/src/sidecar/services/tool_search/jev_ranker.rs`, declared from `tool_search.rs`), plus narrowly scoped ranker configuration/settings if necessary. If a shared tool schema change is needed, write it down for Agent 1; do not concurrently modify the same file.
- Use the existing profile-scoped catalog built by `manager.tool_exposure_for_profile(selected_profile).await`. Never query all discovered tools first and filter later; **no disabled, risk-blocked, other-profile, or unassigned tools may reach OpenRouter or be returned**.
- Preserve the current search signature/behavior at the RPC boundary: `search_for_profile(manager, db, params, agent_info, selected_profile)` returns `SearchOutcome { payload }` with response keys:

```
{
  "tools": [{
    "name": "stable_exposed_name",
    "description": "...",
    "inputSchema": {"type": "object"},
    "serverName": "...",
    "riskLevel": "safe|write|destructive|unknown",
    "serverInstructions": "optional"
  }],
  "nextCursor": "optional"
}
```

- Existing `query`, `server`, `limit`, and `cursor` parameters remain compatible; do not rename `moor_search_tools` to `search_tool`.
- Do not implement or call `moor_mcp_call_tool` here. Search returns exact **authorized, registered tool names** which Agent 1's dispatcher can resolve without remapping.

## Ranking design (smallest reliable change)

1. **Deterministic first pass:** Existing local lexical/alias matching filters and scores tools (including service name, tool name and description), honoring explicit server filter. A query with no useful terms should not call the LLM.
2. **Optional Jev/OpenRouter rerank:** Send a bounded shortlist (e.g. up to 30–50 authorized candidates) with only: stable tool IDs, server names, tool names, short descriptions and the user's search query. Do **not** send credentials, connection URLs, tool input values, per-server private instructions, auth headers, environment variables, database records or whole schemas.
3. **Constrained answer:** Ask the model for a strict JSON array of candidate IDs in preference order. Validate every ID against the authorized shortlist, reject duplicates and unknown IDs, then append any omitted authorized candidates using deterministic fallback order. The model can **rank**, never add/remove permissions or invent tools.
4. **Reliability:** Bound provider request duration (target a few seconds, comfortably within Base bridge's ~25s job timeout), payload size, candidate count, output tokens and cost. On no key, disabled setting, network error, malformed JSON, rate limit, timeout, or unsupported provider: return the **deterministic local ranking**, not an empty list or a bridge error.
5. **Pagination:** Keep `limit <= 50` and `cursor` behavior. Ranked results must not duplicate/drop tools across pages if provider responses vary; use a small scoped ranked-result snapshot or another simple stable ordering strategy, keyed to the profile/query/catalog state. No cross-profile cached results.
6. **Profile and policy:** Strict profile filtering happens before remote ranking. Revalidate policies on tool execution (Agent 1's responsibility). Prompt injection in tool descriptions or the user query must not permit external commands or affect authorization.
7. **Privacy / observability:** Explicitly treat sending a user query/tool catalog excerpt to OpenRouter as an outbound data transfer. Do not log secret values, raw model prompts, arbitrary API response bodies or full tool instructions. Limit audit/metrics to sanitized model, duration, candidate count, fallback reason and provider status as appropriate.

## Jev / OpenRouter credential integration

- The existing Hermes secret source is a **reference for a usable key**, not permission to copy it to source or commit it. Reuse an **approved runtime credential path** for native Moor (prefer existing Moor Keychain-backed credential storage and a reference/config setting). Confirm whether the approved Mac-side path can read an existing secret; **do not add mounts, expose the key through HTTP, create a generic host CLI, or modify Hermes/Controller secrets or policies**.
- Allow opt-in ranking settings in **Moor**, not Base: enabled/disabled, provider/model (configurable), credential reference, and timeout. Default safely to deterministic local ranking when unconfigured. Do not force the repository's unrelated JEV `think` tool into an incompatible response schema.
- Verify actual OpenRouter model identifier at runtime/configuration; don't claim “Jev” is a model ID. Avoid hardcoding an unverified model. The Controller reference defaults to `xiaomi/mimo-v2.6-flash`, but it is only a **reference**, not an authorization to silently change model/provider.
- If the key cannot be supplied securely to the native Moor process through an existing approved mechanism, implement the **configurable adapter + deterministic fallback** and explicitly report the key-delivery blocker. Do not weaken secret boundaries to make a demo work.

## Acceptance / tests

- Authorized tools for `find my Notion documents` prioritize relevant Notion search operations over destructive or irrelevant tools; tests use fake candidate catalogs and mocked ranker responses, not paid live API calls.
- No unauthorized or disabled tool is sent to the ranker or returned, even if the model fabricates its ID.
- Preserve exact `inputSchema`, `serverInstructions`, `riskLevel`, stable `name`, `server` filter and pagination contract.
- Provider timeout, invalid payload, empty key, rate limiting and partial candidate rankings all produce a valid deterministic fallback.
- Isolation across agent profiles; no cache results reused with another profile's tools. Test stable multi-page results.
- Existing `moor_call_tool` and Agent 1's new alias/adapter work without requiring any search-side execution changes. Base OAuth/Convex bridge protocol and local MCP lifecycle remain unchanged.
- Run focused Rust tests and repo checks with approved tooling. No real network call unless explicitly authorized and configured; do not print API keys. If Rust toolchain or Vite native bindings are unavailable in Docker, use documented Mac workflows rather than bypassing unrelated controls. Report verified vs unverified checks.
- Commit implementation only to **your own branch**. Do not merge, push, deploy or alter a running Moor app without explicit instruction. Report the commit ID, model/provider actually used, fallback behavior, verified tests and any credential/configuration blocker.
