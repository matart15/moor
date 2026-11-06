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
