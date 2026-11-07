# Optional Moor-local Jev-style tool ranking

Moor's `moor_search_tools` always starts with the registered, callable
**profile-scoped** tool catalog and deterministic lexical/alias ranking.
JEV is the Controller executive runtime, **not an OpenRouter model or a
verified tool-ranker API**. This optional component directly calls the
OpenRouter Chat Completions endpoint as an advisory sorter; it neither executes
tools nor changes authorization. Agent 1's dispatcher and RPC tool schemas are
untouched.

## Configure in Moor Dev

In **Settings → Advanced → Jev-assisted Tool Ranking**, supply:

- **Enabled** (default off).
- **Provider**: `openrouter` (the only supported provider).
- **Model ID**: an actual OpenRouter model ID verified for the account at
  configuration time (no model is selected by default).
- **Credential reference**: the Keychain account name, default `moor-openrouter`.
  Existing custom references still work; this field never contains a key.
- **OpenRouter API key**: enter it in the masked field and click **Save API key**.
  Moor stores it as a system Keychain password under service
  `com.specdest.moor-dev.tool-ranking`, using the selected account reference.
  The UI only displays whether a key exists, never the saved key itself.
- **Timeout**: 1–8 seconds (default 3000 ms).

The native macOS Moor process reads the Keychain password through the
existing `keyring` library. It **does not** read Docker/Hermes ignored secret
files or Controller environment variables. Hermes's
`secrets/openrouter.env` belongs to the container-side credential boundary:
do not mount, duplicate, commit, display or transmit that secret as a shortcut.
The Keychain password is written through Moor's existing authenticated,
loopback-only settings API. A new key can be saved while ranking is disabled;
it is not sent to OpenRouter until ranking is enabled and a model ID is set.
No host command, mount, environment secret, or credential migration is required.
The key itself is never stored in the settings database, echoed in the API
response, or restored to the password field after saving.

## Network and fallback

When opted in with a model and accessible Keychain credential, Moor sends
**only** the search query (first 400 characters) and up to 40 authorized tool
IDs, names, server labels and short descriptions to
`https://openrouter.ai/api/v1/chat/completions`. This is an **outbound
data transfer** to OpenRouter; it does not include tool schemas, private
server instructions, connection endpoints, arguments, auth headers, stored
secrets or unassigned/disabled/risk-blocked tool metadata. Keep sensitive
information out of search queries and descriptions.

The request is bounded to 16 KiB, the response to 16 KiB, and output to
1024 tokens. The model is asked for an array of IDs; unknown/duplicate IDs,
bad JSON, HTTP failures, timeouts or unavailable credentials produce
deterministic local ranking. Valid _partial_ lists are followed by omitted
authorized candidates in local order. No network call occurs for a query with
no useful terms. Exact tool name, input schema, risk level, server filter and
instructions are returned from Moor's current catalog, never model output.

For stable numeric cursor pagination, an in-memory 5-minute ranking snapshot
stores only ordered exposed tool names, keyed by profile, query, server filter,
catalog content, instructions digest and ranker settings. It contains no
schemas or credentials and is limited to 64 entries. Profile/catalog changes
invalidate the key. After snapshot expiration a new search can produce a new
order, so clients should restart pagination for long-lived searches.

No paid/live provider calls are made by the automated unit tests.
