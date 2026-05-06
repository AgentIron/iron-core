## Why

`iron-core` currently receives a fully constructed provider and has no runtime-owned path for resolving, refreshing, or retrying OAuth-backed provider credentials. This blocks AgentIron and other clients from using OAuth-only providers such as Codex, and dual-mode providers such as `kimi-code`, without duplicating refresh and retry behavior in each application.

## What Changes

- Add provider credential orchestration for inference providers, separate from plugin authentication.
- Add app-owned credential persistence through an `iron-core` credential store abstraction.
- Add OAuth device-code metadata and token exchange/refresh support for `kimi-code` and `codex`.
- Resolve credentials per prompt using app-supplied provider/model selection.
- Build `iron_providers::RuntimeConfig` from either API-key or OAuth bearer credentials before provider invocation.
- Prefer API-key credentials when both API-key and OAuth credentials are configured for a dual-mode provider.
- Expose client-visible provider auth statuses and actionable auth errors.
- Retry once after OAuth provider auth failure when no stream output has been emitted and forced refresh succeeds.
- Preserve existing injected-provider and API-key behavior.

## Capabilities

### New Capabilities
- `provider-credential-orchestration`: Resolve, refresh, report, and apply provider credentials for per-prompt provider execution.

### Modified Capabilities
- None.

## Impact

- Affected code: provider invocation path in `IronRuntime`/`PromptRunner`, facade APIs for managed provider execution, new provider credential domain and OAuth refresh modules, tests for credential resolution and auth retry.
- Affected dependencies: uses existing `iron-providers` credential abstractions (`ProviderCredential`, `RuntimeConfig::from_credential`, provider registry/profile support).
- Affected clients: AgentIron and `iron-tui` can supply provider/model choices per prompt and provide credential store implementations; AgentIron may keep SQLite persistence for V1 as a client-owned backend.
- Non-impact: `iron-core` will not own secure/keyring storage policy, migrate existing AgentIron API-key storage, implement browser redirect OAuth, support multiple accounts per provider, or perform remote token revocation in this change.
