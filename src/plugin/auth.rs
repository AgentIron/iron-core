use serde::{Deserialize, Serialize};

/// OAuth authentication requirements declared by a plugin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthRequirements {
    /// OAuth provider identifier (e.g., "google", "slack", "github")
    pub provider: String,
    /// Human-readable provider name
    pub provider_name: String,
    /// OAuth scopes required by the plugin
    pub scopes: Vec<String>,
    /// Authorization endpoint URL (if using OAuth 2.0)
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    /// Token endpoint URL (if using OAuth 2.0)
    #[serde(default)]
    pub token_endpoint: Option<String>,
}

/// Standardized OAuth providers supported in v1
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    Google,
    GitHub,
    Slack,
    Discord,
    Microsoft,
    Custom,
}

/// Runtime-governed authentication state
///
/// This is the authoritative auth state vocabulary used by iron-core.
/// Plugins declare requirements but do NOT define auth lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// No authentication has been attempted
    Unauthenticated,
    /// Authentication flow is in progress
    Authenticating,
    /// Successfully authenticated with valid credentials
    Authenticated,
    /// Credentials have expired and need refresh
    Expired,
    /// Credentials have been explicitly revoked
    Revoked,
}

impl AuthState {
    /// Check if the current state allows authenticated operations
    pub fn is_authenticated(&self) -> bool {
        matches!(self, Self::Authenticated)
    }

    /// Check if the current state requires re-authentication
    pub fn requires_reauth(&self) -> bool {
        matches!(self, Self::Expired | Self::Revoked | Self::Unauthenticated)
    }

    /// Check if authentication is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(self, Self::Authenticating)
    }
}

/// Auth availability exposed to clients
///
/// This separates runtime health from auth state and provides
/// actionable hints for client UX.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthAvailability {
    /// Current authentication state
    pub state: AuthState,
    /// Whether authentication is required for any tools
    pub auth_required: bool,
    /// Whether the user can initiate authentication
    pub can_authenticate: bool,
    /// User-facing message describing the auth status
    pub message: String,
    /// Action hint for clients (e.g., "start_oauth", "refresh_token", "none")
    pub action_hint: AuthActionHint,
}

/// Hints for client UX on what auth action to present
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthActionHint {
    /// No action needed
    None,
    /// Start OAuth flow
    StartOAuth,
    /// Refresh expired credentials
    RefreshToken,
    /// Re-authenticate after revocation
    Reauthenticate,
}

/// Credential binding stored by the runtime
///
/// This represents an authenticated session with an OAuth provider.
/// The runtime owns this data; plugins do not access credentials directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialBinding {
    /// Plugin ID this credential is for
    pub plugin_id: String,
    /// OAuth provider identifier
    pub provider: String,
    /// Access token (encrypted at rest in production)
    pub access_token: String,
    /// Refresh token (if available)
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Token expiration timestamp (if known)
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Granted scopes
    pub scopes: Vec<String>,
}

/// Client-auth interaction request
///
/// The runtime sends this to clients when authentication is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthInteractionRequest {
    /// Unique request ID
    pub request_id: String,
    /// Plugin ID requiring authentication
    pub plugin_id: String,
    /// OAuth provider
    pub provider: String,
    /// Scopes being requested
    pub scopes: Vec<String>,
    /// Authorization URL for browser launch
    pub authorization_url: String,
    /// Redirect URI to capture
    pub redirect_uri: String,
    /// PKCE code verifier (for secure OAuth)
    #[serde(default)]
    pub code_verifier: Option<String>,
}

/// Structured auth prompt exposed to clients for rendering.
///
/// The runtime produces this from plugin auth requirements and current state
/// so that clients can present a consistent auth UX without routing through
/// the model.  The `auth_id` is the plugin identifier; `state` reflects the
/// current auth lifecycle position; `title` and `description` give the client
/// enough context to render a meaningful prompt.
///
/// # Example
///
/// ```json
/// {
///   "auth_id": "plugin:github",
///   "state": "unauthenticated",
///   "title": "Connect GitHub",
///   "description": "This plugin needs GitHub access before it can continue."
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthPrompt {
    /// Plugin identifier that requires authentication.
    pub auth_id: String,
    /// Current authentication state.
    pub state: AuthState,
    /// Human-readable title for the auth prompt (e.g. "Connect GitHub").
    pub title: String,
    /// Explanation of why authentication is needed.
    pub description: String,
}

/// Notification that an auth state transition occurred.
///
/// Emitted by the runtime whenever a plugin's auth state changes so that
/// clients and sessions can observe the transition without polling.
///
/// # Example
///
/// ```json
/// {
///   "auth_id": "plugin:github",
///   "previous_state": "unauthenticated",
///   "new_state": "authenticated"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthStatusTransition {
    /// Plugin identifier whose auth state changed.
    pub auth_id: String,
    /// Auth state before the transition.
    pub previous_state: AuthState,
    /// Auth state after the transition.
    pub new_state: AuthState,
}

/// Client-auth interaction response
///
/// Clients send this back to the runtime after user interaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthInteractionResponse {
    /// Request ID this responds to
    pub request_id: String,
    /// Result of the authentication flow
    pub result: AuthInteractionResult,
}

/// Result of client authentication interaction
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthInteractionResult {
    /// User successfully authorized
    Success {
        /// Authorization code from redirect
        code: String,
        /// State parameter from redirect (if used)
        #[serde(default)]
        state: Option<String>,
    },
    /// User denied authorization
    Denied {
        /// Error message or code
        reason: String,
    },
    /// Authentication flow failed
    Failed {
        /// Error message
        error: String,
    },
    /// Authentication was cancelled
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_is_authenticated() {
        assert!(AuthState::Authenticated.is_authenticated());
        assert!(!AuthState::Unauthenticated.is_authenticated());
        assert!(!AuthState::Expired.is_authenticated());
    }

    #[test]
    fn test_auth_state_requires_reauth() {
        assert!(AuthState::Unauthenticated.requires_reauth());
        assert!(AuthState::Expired.requires_reauth());
        assert!(AuthState::Revoked.requires_reauth());
        assert!(!AuthState::Authenticated.requires_reauth());
        assert!(!AuthState::Authenticating.requires_reauth());
    }
}
