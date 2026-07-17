//! Provider credential orchestration for inference providers.
//!
//! This module provides the domain types, store boundary, OAuth metadata,
//! credential resolution, provider construction, and auth-failure retry logic
//! needed for `iron-core` to manage OAuth-backed provider credentials without
//! duplicating that logic in every client.
//!
//! ## Store backends
//!
//! Three store implementations are provided:
//!
//! - **`InMemoryCredentialStore`** — infallible in-memory store for tests and
//!   lightweight callers. Credentials are lost when the process exits.
//! - **`NullCredentialStore`** — always returns `None`. Useful when credential
//!   management is handled externally.
//! - **`DurableCredentialStore`** — SQLite-backed persistent store using
//!   `ConfigStore` from `iron_core::config`. Credentials are encrypted at rest
//!   with XChaCha20-Poly1305.
//!
//! ## One-credential-per-provider semantics
//!
//! The durable store enforces at most one credential per provider slug.
//! Storing an API-key credential replaces any existing OAuth credential for
//! that provider, and vice versa. This simplifies the mental model: a provider
//! is either authenticated via API key or OAuth, never both simultaneously.
//!
//! OAuth disconnect removes the stored OAuth credential but leaves any
//! API-key credential unchanged. It does not restore API keys previously
//! replaced by OAuth.
//!
//! ## API-key precedence
//!
//! For dual-mode providers (e.g. `kimi-code`), an explicitly provided API key
//! takes precedence over any stored OAuth credential. This allows callers to
//! override stored credentials on a per-request basis.
//!
//! The module is additive: existing injected-provider paths in `IronAgent` and
//! `IronRuntime` continue to work without using any of these types.
//!
//! ```
//! use iron_core::provider_credential::{
//!     CredentialMode, InMemoryCredentialStore, StoredCredential,
//! };
//! use std::collections::HashMap;
//!
//! let credentials = HashMap::from([
//!     ("openai".into(), StoredCredential::ApiKey("sk-example".into())),
//! ]);
//! let store = InMemoryCredentialStore::from_map(credentials);
//! let snapshot = store.snapshot();
//! assert_eq!(snapshot["openai"].mode(), CredentialMode::ApiKey);
//! ```

pub mod domain;
pub mod durable_store;
pub mod oauth;
pub mod resolver;
pub mod store;

pub use domain::{
    CredentialMode, OAuthTokenSet, ProviderAuthError, ProviderAuthResult, ProviderAuthStatus,
    ProviderPromptContext, ProviderSlug, ResolvedCredential, StoredCredential,
};
pub use durable_store::{CredentialMetadata, DurableCredentialStore, FallibleCredentialStore};
pub use oauth::{
    poll_token_exchange, refresh_access_token, start_device_code_flow, v1_oauth_metadata,
    DeviceCodeInteraction, DeviceCodeStartResult, OAuthFlowKind, OAuthProviderMetadata,
    TokenExchangeResult,
};
pub use resolver::{CredentialResolver, CredentialSupport, REFRESH_MARGIN};
pub use store::{
    DynCredentialStore, InMemoryCredentialStore, NullCredentialStore, ProviderCredentialStore,
};
