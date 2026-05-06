## Context

`iron-core` currently receives an already-built provider through `IronAgent::new(config, provider)` and stores it as a static `Arc<dyn Provider>`. Prompt execution calls that provider directly, so core has no opportunity to resolve current credentials, refresh OAuth tokens, rebuild provider runtime configuration, or retry provider auth failures.

`iron-providers` now exposes credential-aware runtime construction through `ProviderCredential`, `CredentialKind`, `RuntimeConfig::from_credential`, provider profile credential metadata, and built-in profiles for OAuth-capable `kimi-code` and OAuth-only `codex`. It intentionally does not own login UX, refresh-token storage, refresh lifecycle, or provider auth retry.

AgentIron currently stores provider API keys in its own SQLite settings data, while `iron-tui` reads API keys from flags or environment variables. This change should not move those storage policies into `iron-core`; core needs an abstraction so clients own persistence and can later choose stronger storage without changing the orchestration API.

## Goals / Non-Goals

**Goals:**
- Resolve provider credentials per prompt from app-supplied provider/model context.
- Support one OAuth credential per provider slug for V1.
- Keep provider selection and credential persistence owned by clients.
- Let `iron-core` own OAuth provider metadata, device-code exchange helpers, refresh lifecycle, provider auth status, credential resolution, provider construction, and safe retry orchestration.
- Preserve existing injected-provider and API-key behavior.
- Support `kimi-code` OAuth device-code credentials and `codex` OpenAI device-code credentials.
- Pass only `iron_providers::ProviderCredential::OAuthBearer` access-token material, expiry, and optional ID token into `iron-providers`.

**Non-Goals:**
- Moving existing AgentIron API-key storage into the new provider credential store.
- Making `iron-core` responsible for SQLite, OS keyring, encryption, or secure storage policy.
- Supporting multiple accounts or multiple credentials per provider.
- Implementing browser redirect OAuth in V1.
- Performing remote token revocation on disconnect.
- Reusing plugin auth state/storage directly for provider credentials.

## Decisions

### Split OAuth ownership between clients and core
Clients own UX, provider/model selection, persistence backend, and rendering connect/disconnect flows. `iron-core` owns provider OAuth metadata, device-code token exchange helpers, refresh, status derivation, credential resolution, provider construction, and retry orchestration.

This is preferable to app-only OAuth because refresh and retry behavior would otherwise be duplicated across AgentIron and `iron-tui`. It is preferable to core-owned UX because terminal and desktop clients need different interaction surfaces.

### Use an app-supplied provider credential store abstraction
Introduce a core trait or equivalent boundary for reading, writing, listing, and removing provider OAuth credentials by provider slug. AgentIron can back this with SQLite for V1, but that implementation remains client-owned. Future OS keyring storage is a client concern behind the same boundary.

This keeps long-lived provider secret storage out of normal `Config` and avoids declaring SQLite to be secure storage. Existing AgentIron API-key settings remain unchanged for this change.

### Resolve credentials per prompt
The managed provider path should accept app-supplied provider/model context for each prompt or request. Credential resolution happens at prompt execution time, not only when a session is constructed, so OAuth expiry and refresh are handled before each provider call.

This is preferable to a globally selected provider in `iron-core` because provider selection is application-owned. It is preferable to a one-time session provider construction because `iron-providers` bakes auth headers into the HTTP client at construction time.

### Keep injected-provider compatibility
The current `IronAgent::new(config, provider)` path remains valid for callers that construct providers themselves. The new managed-provider path is additive and uses `iron-providers` registry/profile construction only when the client opts into core credential orchestration.

This avoids breaking existing API-key users and tests while allowing AgentIron to adopt provider credential orchestration incrementally.

### Prefer API keys over OAuth for dual-mode providers
For providers that support both API key and OAuth, such as `kimi-code`, API-key configuration wins when both credential modes are available. OAuth is used only when no API key is available for that provider.

This preserves existing behavior and avoids surprising users who already configured API keys.

### Model provider auth statuses explicitly
Expose a minimal client-visible provider auth status set:
- `NotConfigured`
- `ConfiguredApiKey`
- `ConnectedOAuth { expires_at }`
- `Refreshing`
- `Expired`
- `RefreshFailed { reason }`
- `Revoked`
- `UnsupportedCredential`

These statuses are provider-credential statuses, not plugin auth statuses. They may use similar vocabulary, but they belong to inference provider execution.

### Refresh five minutes before expiry
OAuth credentials with an expiry should be refreshed when expired or within five minutes of expiry. The margin can be internal for V1 rather than a public configuration option.

This avoids starting provider requests with tokens that are likely to expire mid-request.

### Retry only safe OAuth auth failures
After a provider auth failure, core may force-refresh an OAuth credential and retry exactly once. Retry is allowed only when the request has not emitted streaming output yet. API-key credentials are not retried through refresh.

This avoids hiding repeated auth failures, avoids duplicate partial streaming output, and keeps provider-specific 401 recovery out of `iron-providers`.

### Preserve optional Codex ID token routing metadata
When OpenAI auth returns an ID token for Codex, core stores it with OAuth credential material and passes it to `iron-providers` as part of `OAuthBearer`. `iron-providers` can derive `chatgpt-account-id` routing metadata from it. Missing ID token does not prevent credential use, but Codex auth/routing errors should mention missing ID token when relevant.

This treats the ID token as optional routing metadata while still preserving it when available.

## Risks / Trade-offs

- [Client-owned storage can be less secure than OS keyring] -> Keep storage behind a core abstraction and document that SQLite is an AgentIron V1 policy, not secure storage guaranteed by `iron-core`.
- [Per-prompt provider construction may add overhead] -> Build the simplest correct path first; only cache provider instances later if profiling shows a real cost and cache invalidation remains tied to credential expiry.
- [Retrying auth failures can duplicate side effects] -> Retry only before stream output is emitted and only once after forced OAuth refresh.
- [OpenAI/Codex auth behavior may require ID-token/account routing details] -> Store/pass ID token when returned and surface targeted Codex auth/routing errors if it is absent.
- [Plugin auth and provider auth terminology can blur] -> Keep provider credential modules and types separate from plugin auth storage and state.

## Migration Plan

- Add provider credential domain types, store/refresher traits, and OAuth provider metadata without changing the existing injected-provider path.
- Add managed provider execution APIs that accept provider/model context per prompt and use the credential resolver before provider construction.
- Update AgentIron or test clients to provide a credential store implementation separately from existing API-key settings.
- Preserve current API-key tests and add OAuth resolver/refresh/retry tests around the managed-provider path.
- Keep rollback simple by leaving the existing injected-provider path available.

## Open Questions

- Should provider OAuth metadata remain hardcoded in `iron-core` after V1, or should a future non-secret metadata contract be added near provider profiles without moving refresh-token handling into `iron-providers`?
