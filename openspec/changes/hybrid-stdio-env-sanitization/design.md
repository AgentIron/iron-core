## Context

`StdioMcpClient` currently spawns MCP server subprocesses with `env_clear()` followed by an allowlist of "safe" environment variables (`PATH`, `HOME`, `SYSTEMROOT`, etc.). This approach breaks tools that depend on vars not in the allowlist — notably `npx` on Windows (needs `APPDATA`, `LOCALAPPDATA`, `USERPROFILE`) and various Linux desktop tools (need `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, etc.). Each new ecosystem requires extending the allowlist, creating ongoing maintenance burden.

The allowlist does not meaningfully improve security because users can already inject arbitrary env vars through MCP server config. The real threat is accidental leakage of secrets (API keys, tokens, cloud credentials) into subprocess environments, not the presence of toolchain configuration vars.

## Goals / Non-Goals

**Goals:**
- Make MCP stdio servers work with common toolchains (Node.js/npx, Python, Rust, Go, etc.) without requiring users to manually configure environment variables.
- Continue to protect against accidental leakage of sensitive environment variables (API keys, auth tokens, cloud credentials) into MCP server subprocesses.
- Make the stripping behavior observable via debug logging.
- Preserve the existing user-configured env override mechanism.

**Non-Goals:**
- Protecting against malicious MCP servers that the user deliberately configured. If a user configures a malicious server, it already has code execution via the subprocess.
- Implementing a configurable blocklist. The sensitive patterns are compiled into the binary. Users who need to pass a sensitive-named var can do so via MCP server config.
- Changing HTTP or HTTP+SSE transport behavior (they don't spawn subprocesses).

## Decisions

### Use a pattern-based blocklist instead of a value-based allowlist
Replace `env_clear()` + `SAFE_ENV_VARS` with inherit-all + strip-sensitive-patterns. The blocklist matches on environment variable name patterns commonly associated with secrets and credentials.

This is preferable to extending the allowlist because it inverts the maintenance burden: instead of chasing every toolchain's required vars, we only need to track known-sensitive naming conventions. New tools work by default; only secrets are removed.

### Match on case-insensitive suffix and prefix patterns
Sensitive env vars follow predictable naming conventions. The blocklist uses two categories:
- **Suffix patterns**: vars ending in `_API_KEY`, `_SECRET`, `_SECRET_KEY`, `_TOKEN`, `_PASSWORD`, `_CREDENTIALS`, `_AUTH_TOKEN`, `_ACCESS_KEY`, `_ACCESS_TOKEN`
- **Exact names**: well-known credential vars like `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AZURE_CLIENT_SECRET`, `GOOGLE_APPLICATION_CREDENTIALS`, `DATABASE_URL`, `GITHUB_TOKEN`, `GH_TOKEN`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`

Matching is case-insensitive because Windows env vars are case-insensitive and some tools use mixed case.

This is preferable to regex because it is simple, fast, and easy to audit.

### Log stripped vars at debug level
When vars are stripped, log the count and names at debug level. This keeps the system debuggable without exposing secret values in logs.

This is preferable to silent stripping because it helps users understand why a tool might fail if a legitimately-named var was caught by the pattern.

### User-configured env overrides everything
The existing MCP server config `env` field is merged last, after stripping. This means users can explicitly pass a var even if it matches a sensitive pattern — they are making a deliberate choice.

This is preferable to stripping user-configured vars because the user config is an explicit, auditable override.

## Risks / Trade-offs

- [A var name matching a sensitive suffix might be a legitimate non-secret var] → The user can re-add it via MCP server config. Debug logging makes this discoverable.
- [New secret naming conventions may emerge over time] → The blocklist is a single compiled list that can be extended in future changes. The blast radius of missing a new convention is smaller than the current blast radius of breaking every unlisted toolchain.
- [Case-insensitive matching on Linux may strip vars that differ only by case from a sensitive pattern] → This is unlikely in practice because Linux env vars are conventionally uppercase, and the patterns target uppercase naming conventions. If it happens, the user config escape hatch applies.
- [Removing `env_clear()` means MCP servers see more of the parent environment] → This is the intended behavior. The parent environment is the user's own shell/desktop environment. MCP servers are user-configured tools, not sandboxed untrusted code.

## Migration Plan

- Replace `inherited_stdio_env()` with a new `sanitized_stdio_env()` function that inherits all parent vars and strips sensitive patterns.
- Update `StdioMcpClient::new()` to use the new function.
- Remove the `SAFE_ENV_VARS` constant.
- Add tests verifying that sensitive vars are stripped and non-sensitive vars are preserved.
- Existing MCP server config `env` continues to work as before (additive/overriding).

## Open Questions

- Should the blocklist be exposed as a configurable list, or is a compiled-in list sufficient for now?
