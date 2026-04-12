## Why

MCP stdio servers that use `npx`, Python, Rust, Go, or other toolchains fail to spawn because `env_clear()` removes environment variables those tools need. The current allowlist approach requires adding each ecosystem's vars one at a time — a never-ending maintenance burden. Meanwhile the allowlist does not meaningfully prevent secret leakage since users can already pass arbitrary env vars through MCP server config.

## What Changes

- Replace the `env_clear()` + allowlist model in `StdioMcpClient` with a hybrid approach: inherit the full parent environment, then strip vars matching sensitive patterns (API keys, tokens, secrets, cloud credentials).
- Log stripped vars at debug level so operators can see what was removed.
- Merge user-configured env vars on top last, so users can re-add or override if needed.
- Remove the `inherited_stdio_env()` function and its `SAFE_ENV_VARS` allowlist.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `session-scoped-mcp-support`: refine the stdio transport subprocess environment handling so that the child process inherits the parent environment minus sensitive patterns rather than a hardcoded allowlist.

## Impact

- Affected code: `src/mcp/client.rs` — `inherited_stdio_env()` and `StdioMcpClient::new()`.
- Affected systems: MCP stdio transport subprocess spawning on all platforms.
- API/protocol impact: no public API changes. MCP server config env remains additive/overriding as before.
