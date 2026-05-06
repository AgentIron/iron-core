## 1. Provider credential domain

- [x] 1.1 Add provider credential modules and public domain types for provider IDs, credential mode, OAuth token material, credential status, and auth errors
- [x] 1.2 Add an app-supplied provider credential store boundary for OAuth lookup, update, removal, and status listing by provider slug
- [x] 1.3 Ensure refresh tokens and stored OAuth secret material are never represented in normal `Config` provider settings
- [x] 1.4 Add unit tests for store-boundary behavior using an in-memory credential store

## 2. OAuth metadata and token lifecycle

- [x] 2.1 Add V1 OAuth provider metadata for `kimi-code` device-code auth against Kimi auth
- [x] 2.2 Add V1 OAuth provider metadata for `codex` device-code auth against OpenAI auth
- [x] 2.3 Implement device-code start, polling/token exchange, and refresh helpers behind core-owned provider OAuth interfaces
- [x] 2.4 Preserve optional Codex ID tokens in stored OAuth material and propagated bearer credentials
- [x] 2.5 Add tests for successful device-code completion, refresh success, refresh failure, and ID-token preservation

## 3. Credential resolution

- [x] 3.1 Add a credential resolver that accepts app-supplied provider/model context per prompt
- [x] 3.2 Resolve API-key credentials without changing existing API-key behavior
- [x] 3.3 Resolve OAuth credentials from the credential store and refresh when expired or within the five-minute refresh margin
- [x] 3.4 Prefer API-key credentials over OAuth credentials for dual-mode providers such as `kimi-code`
- [x] 3.5 Return actionable missing, expired, refresh-failed, revoked, and unsupported-credential errors
- [x] 3.6 Add resolver tests for missing credentials, API-key compatibility, API-key-over-OAuth precedence, near-expiry refresh, refresh failure, and unsupported credential mode

## 4. Managed provider execution path

- [x] 4.1 Add an additive managed-provider runtime/facade path that keeps the existing injected-provider constructor working
- [x] 4.2 Build `iron_providers::RuntimeConfig` from resolved `ProviderCredential::ApiKey` or `ProviderCredential::OAuthBearer`
- [x] 4.3 Construct `GenericProvider` from the `iron-providers` registry/profile after credential resolution
- [x] 4.4 Ensure refresh tokens are never passed into `iron-providers`
- [x] 4.5 Add integration-style tests proving managed provider construction works for API-key, `kimi-code` OAuth, and `codex` OAuth credentials

## 5. Provider auth status and disconnect

- [x] 5.1 Expose provider auth statuses: `NotConfigured`, `ConfiguredApiKey`, `ConnectedOAuth`, `Refreshing`, `Expired`, `RefreshFailed`, `Revoked`, and `UnsupportedCredential`
- [x] 5.2 Add facade/runtime APIs for clients to query provider auth status without exposing secret material
- [x] 5.3 Add OAuth disconnect support that removes OAuth credential material through the store boundary
- [x] 5.4 Ensure OAuth disconnect does not remove API-key configuration
- [x] 5.5 Add tests for status derivation, disconnect/removal, and API-key preservation after OAuth disconnect

## 6. OAuth auth-failure retry

- [x] 6.1 Detect provider auth failures from the managed provider execution path and map them into provider auth errors
- [x] 6.2 Force-refresh and retry exactly once for OAuth-backed auth failures before any stream output is emitted
- [x] 6.3 Do not retry after streamed output has been emitted
- [x] 6.4 Do not OAuth-refresh retry API-key-backed provider auth failures
- [x] 6.5 Add tests for successful retry after forced refresh, retry suppression after output, retry suppression for API-key credentials, and repeated auth failure reporting

## 7. Verification

- [x] 7.1 Run unit tests covering provider credential resolution, OAuth refresh paths, disconnect/removal, API-key compatibility, and retry behavior
- [x] 7.2 Run existing provider construction and prompt runner regression tests to confirm injected-provider behavior is unchanged
- [x] 7.3 Run `cargo test` for `iron-core`
- [x] 7.4 Update public documentation or examples for managed provider credential orchestration if new facade APIs are exposed
