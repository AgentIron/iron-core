//! Provider credential orchestration for inference providers.
//!
//! This module provides the domain types, store boundary, OAuth metadata,
//! credential resolution, provider construction, and auth-failure retry logic
//! needed for `iron-core` to manage OAuth-backed provider credentials without
//! duplicating that logic in every client.
//!
//! The module is additive: existing injected-provider paths in `IronAgent` and
//! `IronRuntime` continue to work without using any of these types.

pub mod domain;
pub mod oauth;
pub mod resolver;
pub mod store;

pub use domain::{
    CredentialMode, OAuthTokenSet, ProviderAuthError, ProviderAuthResult, ProviderAuthStatus,
    ProviderPromptContext, ProviderSlug, ResolvedCredential, StoredCredential,
};
pub use oauth::{
    poll_token_exchange, refresh_access_token, start_device_code_flow, v1_oauth_metadata,
    DeviceCodeInteraction, DeviceCodeStartResult, OAuthProviderMetadata, TokenExchangeResult,
};
pub use resolver::{CredentialResolver, CredentialSupport, REFRESH_MARGIN};
pub use store::{
    DynCredentialStore, InMemoryCredentialStore, NullCredentialStore, ProviderCredentialStore,
};
