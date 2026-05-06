//! OAuth device-code metadata and token lifecycle for V1 provider credentials.
//!
//! This module contains hardcoded OAuth metadata for `kimi-code` and `codex`
//! providers, plus helpers for the device-authorization-grant flow and token
//! refresh. Clients own the login UX; core owns the metadata and exchange logic.

use crate::provider_credential::domain::{OAuthTokenSet, ProviderAuthError, ProviderSlug};
use serde::Deserialize;
use std::time::{Duration, SystemTime};

/// V1 OAuth metadata for a provider.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthProviderMetadata {
    /// Provider slug this metadata applies to.
    pub slug: ProviderSlug,
    /// OAuth issuer base URL (e.g. `https://auth.kimi.com`).
    pub issuer: String,
    /// Device authorization endpoint (relative or absolute).
    pub device_authorization_endpoint: String,
    /// Token endpoint (relative or absolute).
    pub token_endpoint: String,
    /// OAuth client ID.
    pub client_id: String,
    /// Requested scopes.
    pub scopes: Vec<String>,
}

impl OAuthProviderMetadata {
    /// Resolve the full device authorization URL.
    pub fn device_authorization_url(&self) -> String {
        resolve_url(&self.issuer, &self.device_authorization_endpoint)
    }

    /// Resolve the full token URL.
    pub fn token_url(&self) -> String {
        resolve_url(&self.issuer, &self.token_endpoint)
    }
}

fn resolve_url(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", base.trim_end_matches('/'), path)
    }
}

/// Hardcoded V1 metadata for supported OAuth providers.
pub fn v1_oauth_metadata(slug: &ProviderSlug) -> Option<OAuthProviderMetadata> {
    match slug.as_str() {
        "kimi-code" => Some(OAuthProviderMetadata {
            slug: slug.clone(),
            issuer: "https://auth.kimi.com".to_string(),
            device_authorization_endpoint: "/oauth/device/code".to_string(),
            token_endpoint: "/oauth/token".to_string(),
            client_id: "kimi-code-device".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
        }),
        "codex" => Some(OAuthProviderMetadata {
            slug: slug.clone(),
            issuer: "https://auth.openai.com".to_string(),
            device_authorization_endpoint: "/oauth/device/code".to_string(),
            token_endpoint: "/oauth/token".to_string(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
        }),
        _ => None,
    }
}

/// Data returned to the client to start a device-code login interaction.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCodeInteraction {
    /// URL the user should visit to authorize.
    pub verification_uri: String,
    /// Code the user should enter at the verification URI.
    pub user_code: String,
    /// How long the device code is valid (seconds).
    pub expires_in_secs: u64,
    /// Minimum polling interval (seconds).
    pub interval_secs: u64,
}

/// Start a device-code authorization flow.
///
/// Calls the provider's device authorization endpoint and returns the
/// interaction data the client should present to the user.
fn encode_form(params: &[(&str, &str)]) -> String {
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in params {
        encoded.append_pair(k, v);
    }
    encoded.finish()
}

fn refresh_response_error(
    metadata: &OAuthProviderMetadata,
    status: reqwest::StatusCode,
    body: &str,
) -> ProviderAuthError {
    let body_lower = body.to_lowercase();
    if body_lower.contains("invalid_grant")
        || body_lower.contains("revoked")
        || body_lower.contains("access_denied")
    {
        ProviderAuthError::Revoked(metadata.slug.as_str().to_string())
    } else {
        ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("refresh failed ({}): {}", status, body),
        }
    }
}

pub async fn start_device_code_flow(
    metadata: &OAuthProviderMetadata,
    client: &reqwest::Client,
) -> Result<DeviceCodeStartResult, ProviderAuthError> {
    let body = encode_form(&[
        ("client_id", metadata.client_id.as_str()),
        ("scope", metadata.scopes.join(" ").as_str()),
    ]);

    let response = client
        .post(metadata.device_authorization_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("device-code start request failed: {}", e),
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to read device-code response: {}", e),
        })?;

    if !status.is_success() {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("device-code start failed ({}): {}", status, body),
        });
    }

    let parsed: DeviceCodeResponse =
        serde_json::from_str(&body).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to parse device-code response: {}", e),
        })?;

    Ok(DeviceCodeStartResult {
        device_code: parsed.device_code,
        interaction: DeviceCodeInteraction {
            verification_uri: parsed.verification_uri,
            user_code: parsed.user_code,
            expires_in_secs: parsed.expires_in,
            interval_secs: parsed.interval.unwrap_or(5),
        },
    })
}

/// Result of starting a device-code flow.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCodeStartResult {
    /// Device code used for polling (not user-visible).
    pub device_code: String,
    /// Interaction data for the client to render.
    pub interaction: DeviceCodeInteraction,
}

/// Poll the token endpoint to exchange a device code for tokens.
///
/// This should be called repeatedly (respecting `interval_secs`) until it
/// returns either tokens or a terminal error.
pub async fn poll_token_exchange(
    metadata: &OAuthProviderMetadata,
    device_code: &str,
    client: &reqwest::Client,
) -> Result<TokenExchangeResult, ProviderAuthError> {
    let body = encode_form(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", device_code),
        ("client_id", metadata.client_id.as_str()),
    ]);

    let response = client
        .post(metadata.token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("token exchange request failed: {}", e),
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to read token response: {}", e),
        })?;

    // Even on "pending" the token endpoint may return 400 with a known error
    let parsed: TokenResponse =
        serde_json::from_str(&body).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to parse token response: {}", e),
        })?;

    if let Some(error) = parsed.error {
        let reason = match error.as_str() {
            "authorization_pending" => "authorization pending".to_string(),
            "slow_down" => "polling too fast".to_string(),
            "expired_token" => "device code expired".to_string(),
            "access_denied" => "access denied by user".to_string(),
            _ => format!("token exchange error: {}", error),
        };
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason,
        });
    }

    if !status.is_success() {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("token exchange failed ({}): {}", status, body),
        });
    }

    let access_token = parsed
        .access_token
        .ok_or_else(|| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: "token response missing access_token".to_string(),
        })?;

    let expires_at = parsed
        .expires_in
        .map(|secs| SystemTime::now() + Duration::from_secs(secs));

    Ok(TokenExchangeResult {
        access_token,
        refresh_token: parsed.refresh_token.unwrap_or_default(),
        expires_at,
        id_token: parsed.id_token,
    })
}

/// Refresh an OAuth access token using a refresh token.
pub async fn refresh_access_token(
    metadata: &OAuthProviderMetadata,
    refresh_token: &str,
    client: &reqwest::Client,
) -> Result<TokenExchangeResult, ProviderAuthError> {
    let body = encode_form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", metadata.client_id.as_str()),
    ]);

    let response = client
        .post(metadata.token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("refresh request failed: {}", e),
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to read refresh response: {}", e),
        })?;

    if !status.is_success() {
        return Err(refresh_response_error(metadata, status, &body));
    }

    let parsed: TokenResponse =
        serde_json::from_str(&body).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to parse refresh response: {}", e),
        })?;

    let access_token = parsed
        .access_token
        .ok_or_else(|| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: "refresh response missing access_token".to_string(),
        })?;

    let expires_at = parsed
        .expires_in
        .map(|secs| SystemTime::now() + Duration::from_secs(secs));

    Ok(TokenExchangeResult {
        access_token,
        refresh_token: parsed.refresh_token.unwrap_or_default(),
        expires_at,
        id_token: parsed.id_token,
    })
}

/// Result of a successful token exchange or refresh.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenExchangeResult {
    /// New access token.
    pub access_token: String,
    /// New refresh token (may be empty if the server did not return one).
    pub refresh_token: String,
    /// When the access token expires.
    pub expires_at: Option<SystemTime>,
    /// Optional ID token.
    pub id_token: Option<String>,
}

impl TokenExchangeResult {
    /// Convert into an `OAuthTokenSet`.
    ///
    /// If `refresh_token` is empty, the caller should reuse the existing one.
    pub fn into_token_set(self, existing_refresh_token: Option<String>) -> OAuthTokenSet {
        let refresh_token = if self.refresh_token.is_empty() {
            existing_refresh_token.unwrap_or_default()
        } else {
            self.refresh_token
        };
        OAuthTokenSet {
            access_token: self.access_token,
            refresh_token,
            expires_at: self.expires_at,
            id_token: self.id_token,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal JSON types for OAuth responses
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn v1_metadata_kimi_code() {
        let meta = v1_oauth_metadata(&ProviderSlug::new("kimi-code")).unwrap();
        assert_eq!(meta.issuer, "https://auth.kimi.com");
        assert_eq!(meta.client_id, "kimi-code-device");
        assert_eq!(
            meta.device_authorization_url(),
            "https://auth.kimi.com/oauth/device/code"
        );
        assert_eq!(meta.token_url(), "https://auth.kimi.com/oauth/token");
    }

    #[test]
    fn v1_metadata_codex() {
        let meta = v1_oauth_metadata(&ProviderSlug::new("codex")).unwrap();
        assert_eq!(meta.issuer, "https://auth.openai.com");
        assert_eq!(meta.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
    }

    #[test]
    fn v1_metadata_unknown() {
        assert!(v1_oauth_metadata(&ProviderSlug::new("openai")).is_none());
    }

    #[test]
    fn resolve_url_absolute() {
        assert_eq!(
            resolve_url("https://auth.kimi.com", "https://other.com/path"),
            "https://other.com/path"
        );
    }

    #[test]
    fn resolve_url_relative() {
        assert_eq!(
            resolve_url("https://auth.kimi.com", "/oauth/token"),
            "https://auth.kimi.com/oauth/token"
        );
    }

    #[test]
    fn token_exchange_into_token_set_reuses_refresh() {
        let result = TokenExchangeResult {
            access_token: "at".into(),
            refresh_token: "".into(),
            expires_at: None,
            id_token: None,
        };
        let set = result.into_token_set(Some("old_rt".into()));
        assert_eq!(set.refresh_token, "old_rt");
    }

    #[test]
    fn token_exchange_into_token_set_uses_new_refresh() {
        let result = TokenExchangeResult {
            access_token: "at".into(),
            refresh_token: "new_rt".into(),
            expires_at: None,
            id_token: None,
        };
        let set = result.into_token_set(Some("old_rt".into()));
        assert_eq!(set.refresh_token, "new_rt");
    }

    #[test]
    fn device_code_response_parsing() {
        let json = r#"{
            "device_code": "dev123",
            "user_code": "USR-CODE",
            "verification_uri": "https://auth.example.com/verify",
            "expires_in": 600,
            "interval": 5
        }"#;
        let parsed: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.device_code, "dev123");
        assert_eq!(parsed.interval, Some(5));
    }

    #[test]
    fn token_response_parsing_success() {
        let json = r#"{
            "access_token": "at123",
            "refresh_token": "rt456",
            "expires_in": 3600,
            "id_token": "id789"
        }"#;
        let parsed: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.access_token, Some("at123".into()));
        assert_eq!(parsed.id_token, Some("id789".into()));
    }

    #[test]
    fn token_response_parsing_error() {
        let json = r#"{"error": "authorization_pending"}"#;
        let parsed: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.error, Some("authorization_pending".into()));
    }

    async fn serve_once(status: &str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 2048];
            let _ = socket.read(&mut buffer).await.unwrap();
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{}", address)
    }

    fn test_metadata(issuer: String) -> OAuthProviderMetadata {
        OAuthProviderMetadata {
            slug: ProviderSlug::new("codex"),
            issuer,
            device_authorization_endpoint: "/device".into(),
            token_endpoint: "/token".into(),
            client_id: "client".into(),
            scopes: vec!["openid".into(), "offline_access".into()],
        }
    }

    #[tokio::test]
    async fn start_device_code_flow_success() {
        let issuer = serve_once(
            "200 OK",
            r#"{
                "device_code": "device-123",
                "user_code": "USER-123",
                "verification_uri": "https://example.com/verify",
                "expires_in": 600,
                "interval": 7
            }"#,
        )
        .await;
        let metadata = test_metadata(issuer);

        let result = start_device_code_flow(&metadata, &reqwest::Client::new())
            .await
            .unwrap();

        assert_eq!(result.device_code, "device-123");
        assert_eq!(result.interaction.user_code, "USER-123");
        assert_eq!(result.interaction.interval_secs, 7);
    }

    #[tokio::test]
    async fn poll_token_exchange_success() {
        let issuer = serve_once(
            "200 OK",
            r#"{
                "access_token": "access-from-device",
                "refresh_token": "refresh-from-device",
                "expires_in": 3600,
                "id_token": "id-from-device"
            }"#,
        )
        .await;
        let metadata = test_metadata(issuer);

        let result = poll_token_exchange(&metadata, "device-123", &reqwest::Client::new())
            .await
            .unwrap();

        assert_eq!(result.access_token, "access-from-device");
        assert_eq!(result.refresh_token, "refresh-from-device");
        assert_eq!(result.id_token, Some("id-from-device".into()));
        assert!(result.expires_at.is_some());
    }

    #[tokio::test]
    async fn refresh_access_token_success_preserves_id_token_and_reuses_refresh() {
        let issuer = serve_once(
            "200 OK",
            r#"{
                "access_token": "new-access",
                "expires_in": 3600,
                "id_token": "new-id"
            }"#,
        )
        .await;
        let metadata = test_metadata(issuer);

        let result = refresh_access_token(&metadata, "old-refresh", &reqwest::Client::new())
            .await
            .unwrap();
        let token_set = result.into_token_set(Some("old-refresh".into()));

        assert_eq!(token_set.access_token, "new-access");
        assert_eq!(token_set.refresh_token, "old-refresh");
        assert_eq!(token_set.id_token, Some("new-id".into()));
        assert!(token_set.expires_at.is_some());
    }

    #[tokio::test]
    async fn refresh_access_token_failure_reports_refresh_failed() {
        let issuer = serve_once("500 Internal Server Error", r#"{"error":"server_error"}"#).await;
        let metadata = test_metadata(issuer);

        let result = refresh_access_token(&metadata, "old-refresh", &reqwest::Client::new()).await;

        assert!(matches!(
            result,
            Err(ProviderAuthError::RefreshFailed { ref provider, .. }) if provider == "codex"
        ));
    }

    #[tokio::test]
    async fn refresh_access_token_invalid_grant_reports_revoked() {
        let issuer = serve_once("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
        let metadata = test_metadata(issuer);

        let result = refresh_access_token(&metadata, "old-refresh", &reqwest::Client::new()).await;

        assert!(
            matches!(result, Err(ProviderAuthError::Revoked(ref provider)) if provider == "codex")
        );
    }
}
