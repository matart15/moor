# Base connector (Moor Dev)

Built into the existing Rust/Tauri process; no additional daemon or public listener.

Configure via **Settings → Advanced → Base MCP Connector**. Enter the Base Convex deployment URL, connection secret, and enable the connection. Secret storage uses macOS Keychain, not SQLite, source control, or a Docker environment file.

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
6. **Mac powered off or application intentionally quit:** No local task can execute and this local-only app cannot issue an out-of-band alert. A separate cloud-side heartbeat and alert destination would be needed for notification when Moor itself is offline. This is **not yet implemented**.

### Release verification

**Verification (2026-11-07):** The recovery branch passed frontend and native macOS Rust CI tests. A new Apple Silicon Moor Dev bundle was built locally and installed at `~/Applications/Moor Dev.app`. After relaunch, Controller reported `moor_base_mcp` running with Auto-start enabled; a real read-only `project_list` call via Base development Convex → Moor Dev completed successfully. The exact running executable identity was not independently checked. **Still pending:** macOS logout/login and abnormal-exit recovery, network interruption and reconnect testing, OAuth renewal/revocation, production multi-agent profile authorization, and external offline alerting. The updated launch agent must be exercised after the next login; do not intentionally crash the app during unrelated work. Retain the existing OpenAI tunnel until production authorization is verified.
