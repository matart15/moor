# Moor API / CLI adapters

Virtual adapters are stored in Moor's SQLite alongside the existing MCP server,
profile and tool-catalog records. They use **the same profile assignments, tool
deny lists, pins, risk policies, `tools/call` audit and `moor_search_tools`** as
regular MCP tools. `moor_mcp_call_tool` is advertised in curated mode;
`moor_call_tool` remains callable for previous clients. No new MCP listener,
Base configuration or OAuth pairing is required.

## Manage adapters

`/api/adapters` is an owner-only REST management resource protected by Moor's
existing loopback Host and `X-Moor-Token` checks.

| Method | URL                         | Result                                                                               |
| ------ | --------------------------- | ------------------------------------------------------------------------------------ |
| GET    | `/api/adapters`             | List saved definitions, without secrets                                              |
| POST   | `/api/adapters`             | Validate and create definition; return stable `id`                                   |
| GET    | `/api/adapters/{id}`        | Retrieve definition                                                                  |
| PUT    | `/api/adapters/{id}`        | Update definition, preserving `id`/`toolName`                                        |
| DELETE | `/api/adapters/{id}`        | Delete definition and its profile/tool links                                         |
| POST   | `/api/adapters/credentials` | Store `{ "secret": "..." }` in the platform credential store; return `credentialRef` |
| DELETE | `/api/adapters/credentials` | Remove `{ "credentialRef": "adapter:UUID" }` from the credential store               |

Creating an adapter assigns the virtual server only to the currently active
profile. Assign additional profiles and restrict tools through Moor's existing
Profile management. Tool names cannot be changed in place, preserving cached
exposed-name and disabled/pinned policies; create a new adapter for a rename.

### API example

```json
{
  "name": "Internal status API",
  "toolName": "read_status",
  "description": "Read the configured service status",
  "inputSchema": {
    "type": "object",
    "properties": { "service": { "type": "string", "maxLength": 60 } },
    "required": ["service"],
    "additionalProperties": false
  },
  "riskLevel": "safe",
  "adapter": {
    "kind": "api",
    "baseUrl": "https://status.example.com",
    "path": "/v1/status",
    "method": "GET",
    "queryFields": { "service": "service" },
    "bodyFields": [],
    "timeoutMs": 3000,
    "maxResponseBytes": 65536,
    "credentialRef": null
  }
}
```

Only declared parameters may be mapped to the configured query or JSON body.
The agent cannot change URL, path, method, credentials or headers. Responses
are converted to MCP `content` text. HTTPS is required except explicitly
configured `allowLocalHttp: true` local/Docker addresses; DNS is resolved and
pinned per invocation, private targets are blocked for remote HTTPS, and HTTP
redirects are disabled. Requests are capped at 64 KiB; responses and timeouts
have configurable bounded limits. Credentials live in Keychain/secure storage
and cannot be embedded in headers or read through these routes. API secrets are
sent as Bearer tokens. Unsupported JSON-schema keywords are rejected when
creating the tool (supported: required, primitive types, bounds, enum, pattern).

### CLI example (requires separate explicit host-command approval)

```json
{
  "name": "Approved status command",
  "toolName": "status",
  "description": "Read service status through an approved container executor",
  "inputSchema": {
    "type": "object",
    "properties": { "service": { "type": "string", "maxLength": 32 } },
    "required": ["service"],
    "additionalProperties": false
  },
  "riskLevel": "safe",
  "adapter": {
    "kind": "cli",
    "bridgeServerId": "EXISTING_APPROVED_MCP_SERVER_ID",
    "bridgeToolName": "execute_approved_cli",
    "executable": "/opt/approved/bin/status",
    "workingDirectory": "/workplace/project",
    "fixedArgs": ["--json"],
    "argumentFields": ["service"],
    "approvalId": "EXPLICIT_OWNER_APPROVAL_REFERENCE",
    "timeoutMs": 5000,
    "maxOutputBytes": 65536
  }
}
```

The example is **not** pre-registered or approved. CLI adapter definitions
must target an already authorized, container-reachable MCP service implementing
`execute_approved_cli` with **typed argv** and a **separately enforced
executable/approval allowlist**. The executor must verify `approvalId`,
`executable` and `cwd`, use argv-based process spawning (never `bash -c`),
restrict environment and concurrency, and cap runtime and output. Moor checks
that the calling profile can also invoke the configured backing tool and uses
its existing MCP session for the call. Moor never invokes a CLI on the native
Mac host or adds a new host gateway command. There is deliberately no generic
`run_command` fallback. **The CLI path cannot run until an approved typed-argv
container bridge exists and the individual command has explicit owner approval.**

Both adapter types preserve the existing JSON-RPC error format. Adapter audit
rows include profile, tool, origin, duration, sanitized parameter field names,
status and safe error categories; raw API/CLI body content and credentials are
not stored in audit results. This code does not modify Base OAuth, bridge
pairing, MCP-to-MCP transport, existing search/ranking modules or the native
macOS installation.
