## 1. Sensitive pattern matching

- [x] 1.1 Implement the case-insensitive suffix-matching and exact-name blocklist for sensitive environment variable names
- [x] 1.2 Add debug logging that reports the names (not values) of stripped vars

## 2. Replace allowlist with hybrid approach

- [x] 2.1 Replace `inherited_stdio_env()` and `SAFE_ENV_VARS` with a new `sanitized_stdio_env()` function that inherits all parent vars and strips sensitive patterns
- [x] 2.2 Update `StdioMcpClient::new()` to use `sanitized_stdio_env()` instead of `env_clear()` + `inherited_stdio_env()`

## 3. Verification

- [x] 3.1 Add unit tests verifying that sensitive vars (suffix patterns and exact names) are stripped and non-sensitive vars are preserved
- [x] 3.2 Add unit tests verifying case-insensitive matching
- [x] 3.3 Add unit tests verifying that user-configured env vars override stripped values
- [x] 3.4 Run the existing MCP transport and integration test suites to confirm no regressions
