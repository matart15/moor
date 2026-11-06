# Base connector (Moor Dev)

Built into the existing Rust/Tauri process; no additional daemon or public listener.

Configure via **Settings → Advanced → Base MCP Connector**. Enter the Base Convex deployment URL, connection secret, and enable the connection. Secret storage uses macOS Keychain, not SQLite, source control, or a Docker environment file.

The connector uses the official Convex Rust SDK realtime subscription to `moorBridge:pending`. Each job must be atomically claimed before invoking `handle_request_for_profile`, which applies the existing Moor profile/tool restrictions. A claimed job never retries after a disconnect. It sends the result to `moorBridge:complete`.

Base must independently authorize each user + OAuth client to the correct profile (see Base `docs/features/moor-bridge/README.md`). Only `https://*.convex.cloud` deployment URLs are supported. The existing tunnel stays active until native build, installation, network reachability, and live E2E pass.

> Controller compose declares '127.0.0.1:8788:3000' as a loopback-only mapping for Local Coder MCP. This change has not been applied to the running Docker container. After a controlled restart, configure the Local Coder server in Moor with 'http://127.0.0.1:8788/mcp/<token>' (token remains a secret on Mac). Never bind this endpoint to a LAN/public interface.
