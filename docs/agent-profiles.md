# Moor Dev: independent agent profiles

The development fork intentionally does not migrate existing Moor release data.
Its app ID is `com.specdest.moor-dev`, and its default HTTP port is `9323`.

The legacy `http://127.0.0.1:9323/mcp` route still uses the globally active
profile. Do not use that route for separately permissioned agents.

Each profile ID has a dedicated authenticated MCP endpoint:

```
http://127.0.0.1:9323/mcp/profiles/PROFILE_UUID
Authorization: Bearer PROFILE_TOKEN
```

Use the profile management API to create profiles, assign servers and configure
tool permissions. Rotate credentials using
`POST /api/profiles/PROFILE_UUID/access-token` with the Moor management API
authentication header. The API returns the new token **once**, revoking any
previous token for the profile. Only its SHA-256 digest is stored in SQLite.
Every profile endpoint verifies the bearer credential on GET, POST and DELETE,
and MCP sessions are bound to their originating profile.

The restricted `./controller moor` host command handles these operations; it
stores profile tokens on the Mac host with 0600 file permissions and does not
output the secret. A separate authenticated HTTPS tunnel/client configuration
is required for each remote agent. Do not expose `/mcp` or the management API
on the public Internet.

## Building on Apple Silicon

Run `./scripts/build-macos-dev.sh` from the fork's macOS checkout. The script
checks Rust tests and builds a standalone development `.app` using the native
macOS toolchain, without DMG packaging. It does not install or replace released Moor.
