# Base connector (Moor Dev)

Built into the existing Rust/Tauri process; no additional daemon or public listener.

## Moor-managed OAuth pairing (new build; production acceptance pending)

Moor Dev can now pair with Base using the existing `moor_base_mcp` OAuth authorization, without requiring `MOOR_CONNECTION_SECRET` or `MOOR_AGENT_PROFILES` in Base. Base needs the new generic bridge code deployed once.

1. Authorize `moor_base_mcp` with the Base account used by the agent clients (requires the `mcp:write` scope).
2. In **Settings → Advanced → Base MCP Connector**, enter the Base production Convex cloud URL, enable **Moor-managed OAuth pairing**, enable the connector, and restart Moor. A new random per-user connector secret is stored in macOS Keychain and registered through the existing Base OAuth grant.
3. Each agent signs into Base through existing OAuth. The agent identity is forwarded to Moor; newly observed clients appear in connector settings. Assign the desired Moor profile there. Unassigned clients fail closed.
4. Agent/profile/server assignments are changed only in Moor. Pairing renews every 30 minutes; Moor sends a heartbeat every 30 seconds, and Base stops forwarding remote tools after 90 seconds without one. The OAuth pairing lease expires after 90 minutes without renewal.

For large tool catalogs (for example Notion plus Sentry), use the profile’s **Curated listing** mode if `tools/list` exceeds Convex’s 180 KB response limit. Curated mode provides discovery and delegated tool invocation without transferring every schema at once.

The old manually configured secret remains available only in legacy mode for non-disruptive migration. Keep the OpenAI tunnel and existing production routing unchanged until native-build, OAuth, cross-agent isolation and live MCP E2E acceptance complete.

**Legacy configuration:** In Settings → Advanced → Base MCP Connector, disable Moor-managed OAuth pairing, enter the Base Convex URL and shared connection secret, and enable the connector. The secret remains in Keychain.

The connector uses the official Convex Rust SDK realtime subscription to `moorBridge:pending`. Each job must be atomically claimed before invoking `handle_request_for_profile`, which applies the existing Moor profile/tool restrictions. A claimed job never retries after a disconnect. It sends the result to `moorBridge:complete`.

Base must independently authorize each user + OAuth client to the correct profile (see Base `docs/features/moor-bridge/README.md`). Only `https://*.convex.cloud` deployment URLs are supported. Keep the existing OpenAI tunnel active until live production OAuth routing, profile authorization, and permissions pass end-to-end testing.

> **Development verification (2026-11-06):** The native Apple Silicon Moor Dev app successfully subscribed to Convex development, completed ping and tools/list, rejected invalid profiles/tools, and executed a read-only call through its OAuth-authenticated **moor_base_mcp** server to Base production (project_list). This does not establish production Base OAuth agent-to-profile routing or full permission enforcement.
>
> The existing **moor_base_mcp** server targets `https://mild-snail-669.convex.site/mcp` and uses its own OAuth authorization, distinct from the outbound connector secret stored in Keychain. Do not add a duplicate server.
>
> Controller compose declares `127.0.0.1:8788:3000` for Local Coder MCP, but it is **not activated** in the running container. After a controlled restart, register the Local Coder MCP server as `http://127.0.0.1:8788/mcp/<token>`. Keep the token secret, expose no LAN port, and preserve the existing OpenAI tunnel until live production acceptance passes.

## Recovery and monitoring

1. **Start on login / crash recovery:** In **Settings → General**, enable **Auto-start on Login** for the packaged Moor Dev app. Its existing per-user macOS LaunchAgent is amended with `KeepAlive = { SuccessfulExit = false }` and a 10-second throttle. This makes launchd restart Moor after an abnormal exit, but **does not restart it after intentional Quit**. The updated launch agent takes effect at the **next login**; do not add a duplicate supervisor or run original Moor. Auto-start servers require **Auto-start Servers on Launch** and individual server auto-start flags. OAuth consent and credential revocation still require the user.
2. **Transient Convex outage / Wi-Fi / sleep:** The same Rust worker reconnects with capped exponential backoff (5, 10, 20, 40, then 60 seconds), resetting after the first successful subscription result. No claimed request is replayed after connection loss.
3. **Diagnostics:** Authenticated `GET /api/bridge` returns `connection`, `lastConnectedAtMs`, `lastMessageAtMs`, `lastError` (sanitized category), `consecutiveFailures`, `retryAfterSeconds`, and `secretConfigured`. Settings → Advanced refreshes this status every 10 seconds while open, showing the last successful connection and any recurring failures. Repeated `Connection secret rejected by Convex` means the Keychain secret differs from the target deployment; it should be corrected, never logged.
4. **MCP server failure:** Enable **Automatic Health Checks** in Settings → Advanced to detect unhealthy running servers. These checks warn after three failures; server restart is still manual to avoid unintentionally resetting active tool calls. OAuth refresh must be verified separately.
5. **Request timeout / uncertain writes:** Base expires pending requests after 25 seconds. Once claimed, a tool may have executed even if its response was lost. Check state before attempting a write again; do **not** automatically replay it.
6. **Mac powered off or application intentionally quit:** No local task can execute and this local-only app cannot issue an out-of-band alert. The OAuth-paired connector now heartbeats to Base for routing availability, but no external alert destination is configured. Offline notification is **not yet implemented**.

### Release verification

**Verification (2026-11-07):** The recovery branch passed frontend and native macOS Rust CI tests. A new Apple Silicon Moor Dev bundle was built locally and installed at `~/Applications/Moor Dev.app`. After relaunch, Controller reported `moor_base_mcp` running with Auto-start enabled; a real read-only `project_list` call via Base development Convex → Moor Dev completed successfully. The exact running executable identity was not independently checked. **Still pending:** macOS logout/login and abnormal-exit recovery, network interruption and reconnect testing, OAuth renewal/revocation, production multi-agent profile authorization, and external offline alerting. The updated launch agent must be exercised after the next login; do not intentionally crash the app during unrelated work. Retain the existing OpenAI tunnel until production authorization is verified.

## Repeatable macOS development installation

From the **trusted macOS Moor checkout**, run `nps local.installDev`. This uses
`scripts/build-macos-dev.sh` (dependency install, Rust checks, native Tauri build)
then `scripts/install-macos-dev.sh` (verify bundle identifier/signature where
present, quit old Moor Dev, stage and replace `~/Applications/Moor Dev.app`,
clear quarantine on that locally built bundle only, launch, health-check, and
restore the old app if launch/health validation fails). It never changes
`/Applications/Moor.app` or disables Gatekeeper system-wide. Moor Dev remains
unsigned and is not Apple-notarized; only use the workflow with trusted source.

For **Mac Local Coder (Docker)**, the same NPS command must run via the existing
Controller host gateway, not inside Linux. A Mac administrator must first
review and run Controller's `scripts/internal/install-moor-dev-nps-gateway-macos.sh`
to register the Moor project and update the scoped gateway policy. The gateway
allows only `local.installDev` and the read-only `local.diagnoseMoor` for
`moor-dev`, with sufficient timeout for the native build. Diagnostics report
sanitized bridge connection/assignment counts and server log categories without
exposing OAuth credentials, agent identifiers, or raw logs. This one-time host policy step is not performed by a repo pull.
