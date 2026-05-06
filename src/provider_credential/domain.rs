//! Domain types for provider credential orchestration.
//!
//! These types model provider identity, stored credential material, auth status,
//! and actionable errors. They are separate from plugin auth types to avoid
//! terminology confusion.

use std::time::SystemTime;

/// Provider identifier (slug) used for credential lookup and profile resolution.
///
/// Examples: `"kimi-code"`, `"codex"`, `"openai"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderSlug(pub String);

impl ProviderSlug {
    /// Create a new provider slug.
    pub fn new<S: Into<String>>(slug: S) -> Self {
        Self(slug.into())
    }

    /// Borrow the underlying slug string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProviderSlug {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ProviderSlug {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Which credential mode is stored for a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialMode {
    /// API key credential.
    ApiKey,
    /// OAuth bearer credential.
    OAuthBearer,
}

/// OAuth token material stored for a provider.
///
/// This includes the refresh token, which is intentionally not passed to
/// `iron-providers`. Only the access token (and optional ID token) are
/// forwarded after resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthTokenSet {
    /// Current access token.
    pub access_token: String,
    /// Refresh token used to obtain new access tokens.
    pub refresh_token: String,
    /// When the access token expires, if known.
    pub expires_at: Option<SystemTime>,
    /// Optional ID token (e.g. for Codex account routing).
    pub id_token: Option<String>,
}

/// A stored provider credential, including secret material.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredCredential {
    /// API key string.
    ApiKey(String),
    /// OAuth token set with refresh token.
    OAuthBearer(OAuthTokenSet),
}

impl StoredCredential {
    /// Return the credential mode.
    pub fn mode(&self) -> CredentialMode {
        match self {
            StoredCredential::ApiKey(_) => CredentialMode::ApiKey,
            StoredCredential::OAuthBearer(_) => CredentialMode::OAuthBearer,
        }
    }
}

/// Client-visible provider auth status without secret material.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderAuthStatus {
    /// No credential is configured for this provider.
    NotConfigured,
    /// An API key is configured.
    ConfiguredApiKey,
    /// OAuth is connected with a known expiry.
    ConnectedOAuth { expires_at: Option<SystemTime> },
    /// OAuth refresh is in progress.
    Refreshing,
    /// OAuth access token has expired.
    Expired,
    /// OAuth refresh failed with a reason.
    RefreshFailed { reason: String },
    /// OAuth credential was revoked.
    Revoked,
    /// The configured credential mode is not supported by the provider.
    UnsupportedCredential,
}

/// Errors that can occur during credential resolution or refresh.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderAuthError {
    /// No credential is configured for the requested provider.
    #[error("No credential configured for provider '{0}'")]
    NotConfigured(String),

    /// The provider does not support the configured credential mode.
    #[error("Provider '{provider}' does not support credential mode {mode:?}")]
    UnsupportedCredential {
        provider: String,
        mode: CredentialMode,
    },

    /// The OAuth access token has expired and no refresh token is available.
    #[error("OAuth token expired for provider '{0}'")]
    Expired(String),

    /// OAuth token refresh failed.
    #[error("OAuth refresh failed for provider '{provider}': {reason}")]
    RefreshFailed { provider: String, reason: String },

    /// The OAuth credential was revoked.
    #[error("OAuth credential revoked for provider '{0}'")]
    Revoked(String),
}

/// Result type for provider credential operations.
pub type ProviderAuthResult<T> = Result<T, ProviderAuthError>;

/// Context supplied by the client when resolving credentials for a prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPromptContext {
    /// Provider slug (e.g. "kimi-code", "codex").
    pub provider_slug: ProviderSlug,
    /// Model identifier for the prompt.
    pub model: String,
    /// Optional API key supplied by the client. When present and supported by
    /// the provider, this takes precedence over any stored OAuth credential.
    pub api_key: Option<String>,
}

/// A resolved credential ready for provider construction.
///
/// This is the output of credential resolution. It contains the
/// `iron_providers::ProviderCredential` (which never includes refresh tokens)
/// and metadata about the resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCredential {
    /// The provider-safe credential to pass to `iron-providers`.
    pub provider_credential: iron_providers::ProviderCredential,
    /// The original stored credential mode.
    pub mode: CredentialMode,
    /// Whether the credential was refreshed during resolution.
    pub was_refreshed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_slug_from_str() {
        let slug = ProviderSlug::from("kimi-code");
        assert_eq!(slug.as_str(), "kimi-code");
    }

    #[test]
    fn stored_credential_mode_api_key() {
        let cred = StoredCredential::ApiKey("sk-test".into());
        assert_eq!(cred.mode(), CredentialMode::ApiKey);
    }

    #[test]
    fn stored_credential_mode_oauth() {
        let cred = StoredCredential::OAuthBearer(OAuthTokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: None,
            id_token: None,
        });
        assert_eq!(cred.mode(), CredentialMode::OAuthBearer);
    }

    #[test]
    fn provider_auth_status_variants() {
        // Just exercise construction
        let _ = ProviderAuthStatus::NotConfigured;
        let _ = ProviderAuthStatus::ConfiguredApiKey;
        let _ = ProviderAuthStatus::ConnectedOAuth { expires_at: None };
        let _ = ProviderAuthStatus::Refreshing;
        let _ = ProviderAuthStatus::Expired;
        let _ = ProviderAuthStatus::RefreshFailed {
            reason: "network".into(),
        };
        let _ = ProviderAuthStatus::Revoked;
        let _ = ProviderAuthStatus::UnsupportedCredential;
    }

    #[test]
    fn provider_auth_error_display() {
        let e = ProviderAuthError::NotConfigured("codex".into());
        assert!(e.to_string().contains("codex"));
    }
}
