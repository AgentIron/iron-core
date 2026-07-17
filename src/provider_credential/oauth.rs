//! OAuth device-code metadata and token lifecycle for V1 provider credentials.
//!
//! This module contains hardcoded OAuth metadata for `kimi-code` and `codex`
//! providers, plus helpers for the device-authorization-grant flow and token
//! refresh. Clients own the login UX; core owns the metadata and exchange logic.

use crate::provider_credential::domain::{OAuthTokenSet, ProviderAuthError, ProviderSlug};
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CODEX_DEVICE_AUTH_USERCODE_ENDPOINT: &str = "/api/accounts/deviceauth/usercode";
const CODEX_DEVICE_AUTH_TOKEN_ENDPOINT: &str = "/api/accounts/deviceauth/token";
const CODEX_DEVICE_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
const CODEX_DEVICE_AUTH_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const CODEX_ORIGINATOR: &str = "openclaw";
const CODEX_USER_AGENT: &str = "openclaw/iron-core";

/// OAuth flow variant used by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlowKind {
    /// RFC 8628 device-code grant.
    GenericDeviceCode,
    /// OpenAI Codex-specific device auth flow.
    OpenAiCodexDeviceAuth,
}

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
    /// Provider-specific flow implementation.
    pub flow_kind: OAuthFlowKind,
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
            device_authorization_endpoint: "/api/oauth/device_authorization".to_string(),
            token_endpoint: "/api/oauth/token".to_string(),
            client_id: "17e5f671-d194-4dfb-9706-5516cb48c098".to_string(),
            scopes: vec![],
            flow_kind: OAuthFlowKind::GenericDeviceCode,
        }),
        "codex" => Some(OAuthProviderMetadata {
            slug: slug.clone(),
            issuer: "https://auth.openai.com".to_string(),
            device_authorization_endpoint: CODEX_DEVICE_AUTH_USERCODE_ENDPOINT.to_string(),
            token_endpoint: "/oauth/token".to_string(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
            flow_kind: OAuthFlowKind::OpenAiCodexDeviceAuth,
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

/// Start the provider-specific device authorization flow.
///
/// Calls the configured device authorization endpoint and returns opaque
/// polling state plus interaction data that is safe to present to the user.
///
/// # Errors
///
/// Returns [`ProviderAuthError::RefreshFailed`] for request failures,
/// unsuccessful responses, malformed response bodies, or missing required
/// device-auth fields.
pub async fn start_device_code_flow(
    metadata: &OAuthProviderMetadata,
    client: &reqwest::Client,
) -> Result<DeviceCodeStartResult, ProviderAuthError> {
    match metadata.flow_kind {
        OAuthFlowKind::GenericDeviceCode => start_generic_device_code_flow(metadata, client).await,
        OAuthFlowKind::OpenAiCodexDeviceAuth => {
            start_codex_device_auth_flow(metadata, client).await
        }
    }
}

async fn start_generic_device_code_flow(
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

async fn start_codex_device_auth_flow(
    metadata: &OAuthProviderMetadata,
    client: &reqwest::Client,
) -> Result<DeviceCodeStartResult, ProviderAuthError> {
    let response = client
        .post(metadata.device_authorization_url())
        .header(CONTENT_TYPE, "application/json")
        .header("originator", CODEX_ORIGINATOR)
        .header(USER_AGENT, CODEX_USER_AGENT)
        .json(&serde_json::json!({ "client_id": metadata.client_id.as_str() }))
        .send()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex device-auth start request failed: {}", e),
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to read Codex device-auth response: {}", e),
        })?;

    if !status.is_success() {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex device-auth start failed ({}): {}", status, body),
        });
    }

    let parsed: Value =
        serde_json::from_str(&body).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to parse Codex device-auth response: {}", e),
        })?;

    let device_auth_id = parsed
        .get("device_auth_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: "Codex device-auth response missing device_auth_id".to_string(),
        })?
        .to_string();
    let user_code = parsed
        .get("user_code")
        .or_else(|| parsed.get("usercode"))
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: "Codex device-auth response missing user_code".to_string(),
        })?
        .to_string();
    let expires_in_secs = parsed
        .get("expires_in")
        .and_then(Value::as_u64)
        .or_else(|| codex_expires_at_to_secs(parsed.get("expires_at")))
        .unwrap_or(600);
    let state = CodexDeviceAuthState {
        device_auth_id,
        user_code: user_code.clone(),
    };
    let device_code =
        serde_json::to_string(&state).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to encode Codex device-auth state: {}", e),
        })?;

    Ok(DeviceCodeStartResult {
        device_code,
        interaction: DeviceCodeInteraction {
            verification_uri: CODEX_DEVICE_VERIFICATION_URI.to_string(),
            user_code,
            expires_in_secs,
            interval_secs: parsed.get("interval").and_then(Value::as_u64).unwrap_or(5),
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
///
/// # Errors
///
/// Pending authorization and polling throttles are represented as
/// [`ProviderAuthError::RefreshFailed`] with an actionable reason, as are
/// network, protocol, decoding, denial, and expiry failures.
pub async fn poll_token_exchange(
    metadata: &OAuthProviderMetadata,
    device_code: &str,
    client: &reqwest::Client,
) -> Result<TokenExchangeResult, ProviderAuthError> {
    match metadata.flow_kind {
        OAuthFlowKind::GenericDeviceCode => {
            poll_generic_token_exchange(metadata, device_code, client).await
        }
        OAuthFlowKind::OpenAiCodexDeviceAuth => {
            poll_codex_device_auth_exchange(metadata, device_code, client).await
        }
    }
}

async fn poll_generic_token_exchange(
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

    if let Some(error) = parsed.error.as_deref() {
        let reason = match error {
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

    let access_token =
        parsed
            .access_token
            .clone()
            .ok_or_else(|| ProviderAuthError::RefreshFailed {
                provider: metadata.slug.as_str().to_string(),
                reason: "token response missing access_token".to_string(),
            })?;

    Ok(token_response_to_result(parsed, access_token))
}

async fn poll_codex_device_auth_exchange(
    metadata: &OAuthProviderMetadata,
    device_code: &str,
    client: &reqwest::Client,
) -> Result<TokenExchangeResult, ProviderAuthError> {
    let state: CodexDeviceAuthState =
        serde_json::from_str(device_code).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("invalid Codex device-auth state: {}", e),
        })?;

    let response = client
        .post(resolve_url(
            &metadata.issuer,
            CODEX_DEVICE_AUTH_TOKEN_ENDPOINT,
        ))
        .header(CONTENT_TYPE, "application/json")
        .header("originator", CODEX_ORIGINATOR)
        .header(USER_AGENT, CODEX_USER_AGENT)
        .json(&serde_json::json!({
            "device_auth_id": state.device_auth_id,
            "user_code": state.user_code,
        }))
        .send()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex device-auth poll request failed: {}", e),
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to read Codex device-auth poll response: {}", e),
        })?;

    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: "authorization pending".to_string(),
        });
    }

    if !status.is_success() {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex device-auth poll failed ({}): {}", status, body),
        });
    }

    let parsed: CodexAuthorizationResponse =
        serde_json::from_str(&body).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to parse Codex device-auth poll response: {}", e),
        })?;

    exchange_codex_authorization_code(metadata, parsed, client).await
}

async fn exchange_codex_authorization_code(
    metadata: &OAuthProviderMetadata,
    authorization: CodexAuthorizationResponse,
    client: &reqwest::Client,
) -> Result<TokenExchangeResult, ProviderAuthError> {
    let body = encode_form(&[
        ("grant_type", "authorization_code"),
        ("code", authorization.authorization_code.as_str()),
        ("redirect_uri", CODEX_DEVICE_AUTH_REDIRECT_URI),
        ("client_id", metadata.client_id.as_str()),
        ("code_verifier", authorization.code_verifier.as_str()),
    ]);

    let response = client
        .post(metadata.token_url())
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex token exchange request failed: {}", e),
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to read Codex token exchange response: {}", e),
        })?;

    let parsed: TokenResponse =
        serde_json::from_str(&body).map_err(|e| ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("failed to parse Codex token exchange response: {}", e),
        })?;

    if let Some(error) = parsed.error.as_deref() {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex token exchange error: {}", error),
        });
    }

    if !status.is_success() {
        return Err(ProviderAuthError::RefreshFailed {
            provider: metadata.slug.as_str().to_string(),
            reason: format!("Codex token exchange failed ({}): {}", status, body),
        });
    }

    let access_token =
        parsed
            .access_token
            .clone()
            .ok_or_else(|| ProviderAuthError::RefreshFailed {
                provider: metadata.slug.as_str().to_string(),
                reason: "Codex token response missing access_token".to_string(),
            })?;

    Ok(token_response_to_result(parsed, access_token))
}

/// Refresh an OAuth access token using a refresh token.
///
/// # Errors
///
/// Returns [`ProviderAuthError::Revoked`] for revoked or invalid grants and
/// [`ProviderAuthError::RefreshFailed`] for transport, response-status,
/// decoding, or missing-token failures.
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

    let access_token =
        parsed
            .access_token
            .clone()
            .ok_or_else(|| ProviderAuthError::RefreshFailed {
                provider: metadata.slug.as_str().to_string(),
                reason: "refresh response missing access_token".to_string(),
            })?;

    Ok(token_response_to_result(parsed, access_token))
}

fn token_response_to_result(parsed: TokenResponse, access_token: String) -> TokenExchangeResult {
    let expires_at = parsed
        .expires_in
        .map(|secs| SystemTime::now() + Duration::from_secs(secs))
        .or_else(|| jwt_expiration(&access_token));

    TokenExchangeResult {
        access_token,
        refresh_token: parsed.refresh_token.unwrap_or_default(),
        expires_at,
        id_token: parsed.id_token,
    }
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

#[derive(Debug, Serialize, Deserialize)]
struct CodexDeviceAuthState {
    device_auth_id: String,
    user_code: String,
}

#[derive(Debug, Deserialize)]
struct CodexAuthorizationResponse {
    authorization_code: String,
    code_verifier: String,
}

fn codex_expires_at_to_secs(value: Option<&Value>) -> Option<u64> {
    let expires_at = value?;
    let expires = if let Some(timestamp) = expires_at.as_u64() {
        UNIX_EPOCH + Duration::from_secs(timestamp)
    } else if let Some(timestamp) = expires_at.as_str() {
        let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
        SystemTime::from(parsed)
    } else {
        return None;
    };
    expires
        .duration_since(SystemTime::now())
        .ok()
        .map(|d| d.as_secs())
}

fn jwt_expiration(access_token: &str) -> Option<SystemTime> {
    let payload = access_token.split('.').nth(1)?;
    let decoded = decode_base64_url(payload)?;
    let parsed: Value = serde_json::from_slice(&decoded).ok()?;
    parsed
        .get("exp")
        .and_then(Value::as_u64)
        .map(|exp| UNIX_EPOCH + Duration::from_secs(exp))
}

fn decode_base64_url(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    #[test]
    fn v1_metadata_kimi_code() {
        let meta = v1_oauth_metadata(&ProviderSlug::new("kimi-code")).unwrap();
        assert_eq!(meta.issuer, "https://auth.kimi.com");
        assert_eq!(meta.client_id, "17e5f671-d194-4dfb-9706-5516cb48c098");
        assert_eq!(
            meta.device_authorization_url(),
            "https://auth.kimi.com/api/oauth/device_authorization"
        );
        assert_eq!(meta.token_url(), "https://auth.kimi.com/api/oauth/token");
        assert!(meta.scopes.is_empty());
    }

    #[test]
    fn v1_metadata_codex() {
        let meta = v1_oauth_metadata(&ProviderSlug::new("codex")).unwrap();
        assert_eq!(meta.issuer, "https://auth.openai.com");
        assert_eq!(meta.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(meta.flow_kind, OAuthFlowKind::OpenAiCodexDeviceAuth);
        assert_eq!(
            meta.device_authorization_url(),
            "https://auth.openai.com/api/accounts/deviceauth/usercode"
        );
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

    async fn serve_requests(
        responses: Vec<(&'static str, String)>,
    ) -> (String, mpsc::UnboundedReceiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 4096];
                let bytes_read = socket.read(&mut buffer).await.unwrap();
                tx.send(String::from_utf8_lossy(&buffer[..bytes_read]).into_owned())
                    .unwrap();
                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{}", address), rx)
    }

    fn test_metadata(issuer: String) -> OAuthProviderMetadata {
        OAuthProviderMetadata {
            slug: ProviderSlug::new("codex"),
            issuer,
            device_authorization_endpoint: "/device".into(),
            token_endpoint: "/token".into(),
            client_id: "client".into(),
            scopes: vec!["openid".into(), "offline_access".into()],
            flow_kind: OAuthFlowKind::GenericDeviceCode,
        }
    }

    fn codex_test_metadata(issuer: String) -> OAuthProviderMetadata {
        OAuthProviderMetadata {
            slug: ProviderSlug::new("codex"),
            issuer,
            device_authorization_endpoint: CODEX_DEVICE_AUTH_USERCODE_ENDPOINT.into(),
            token_endpoint: "/oauth/token".into(),
            client_id: "client".into(),
            scopes: vec!["openid".into(), "offline_access".into()],
            flow_kind: OAuthFlowKind::OpenAiCodexDeviceAuth,
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
    async fn start_codex_device_auth_posts_json_headers_and_returns_interaction() {
        let (issuer, mut requests) = serve_requests(vec![(
            "200 OK",
            r#"{
                    "device_auth_id": "device-auth-123",
                    "usercode": "CODE-123",
                    "expires_in": 900,
                    "interval": 4
                }"#
            .to_string(),
        )])
        .await;
        let metadata = codex_test_metadata(issuer);

        let result = start_device_code_flow(&metadata, &reqwest::Client::new())
            .await
            .unwrap();

        let request = requests.recv().await.unwrap();
        assert!(request.starts_with("POST /api/accounts/deviceauth/usercode HTTP/1.1"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.contains("originator: openclaw"));
        assert!(request.contains("user-agent: openclaw/iron-core"));
        assert!(request.contains(r#"{"client_id":"client"}"#));

        let state: CodexDeviceAuthState = serde_json::from_str(&result.device_code).unwrap();
        assert_eq!(state.device_auth_id, "device-auth-123");
        assert_eq!(state.user_code, "CODE-123");
        assert_eq!(
            result.interaction.verification_uri,
            CODEX_DEVICE_VERIFICATION_URI
        );
        assert_eq!(result.interaction.user_code, "CODE-123");
        assert_eq!(result.interaction.expires_in_secs, 900);
        assert_eq!(result.interaction.interval_secs, 4);
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
    async fn poll_codex_device_auth_treats_403_as_pending() {
        let (issuer, mut requests) = serve_requests(vec![(
            "403 Forbidden",
            r#"{"error":"pending"}"#.to_string(),
        )])
        .await;
        let metadata = codex_test_metadata(issuer);
        let state = serde_json::to_string(&CodexDeviceAuthState {
            device_auth_id: "device-auth-123".into(),
            user_code: "CODE-123".into(),
        })
        .unwrap();

        let result = poll_token_exchange(&metadata, &state, &reqwest::Client::new()).await;

        let request = requests.recv().await.unwrap();
        assert!(request.starts_with("POST /api/accounts/deviceauth/token HTTP/1.1"));
        assert!(request.contains("originator: openclaw"));
        assert!(request.contains(r#""device_auth_id":"device-auth-123""#));
        assert!(request.contains(r#""user_code":"CODE-123""#));
        assert!(matches!(
            result,
            Err(ProviderAuthError::RefreshFailed { reason, .. }) if reason == "authorization pending"
        ));
    }

    #[tokio::test]
    async fn poll_codex_device_auth_exchanges_authorization_code_for_tokens() {
        let jwt_with_exp = "header.eyJleHAiOjQxMDI0NDQ4MDB9.signature";
        let token_body = format!(
            r#"{{"access_token":"{}","refresh_token":"refresh-token","id_token":"id-token"}}"#,
            jwt_with_exp
        );
        let (issuer, mut requests) = serve_requests(vec![
            (
                "200 OK",
                r#"{"authorization_code":"auth-code","code_verifier":"verifier-123"}"#.to_string(),
            ),
            ("200 OK", token_body),
        ])
        .await;
        let metadata = codex_test_metadata(issuer);
        let state = serde_json::to_string(&CodexDeviceAuthState {
            device_auth_id: "device-auth-123".into(),
            user_code: "CODE-123".into(),
        })
        .unwrap();

        let result = poll_token_exchange(&metadata, &state, &reqwest::Client::new())
            .await
            .unwrap();

        let poll_request = requests.recv().await.unwrap();
        let exchange_request = requests.recv().await.unwrap();
        assert!(poll_request.starts_with("POST /api/accounts/deviceauth/token HTTP/1.1"));
        assert!(exchange_request.starts_with("POST /oauth/token HTTP/1.1"));
        assert!(exchange_request.contains("grant_type=authorization_code"));
        assert!(exchange_request.contains("code=auth-code"));
        assert!(exchange_request
            .contains("redirect_uri=https%3A%2F%2Fauth.openai.com%2Fdeviceauth%2Fcallback"));
        assert!(exchange_request.contains("client_id=client"));
        assert!(exchange_request.contains("code_verifier=verifier-123"));

        assert_eq!(result.access_token, jwt_with_exp);
        assert_eq!(result.refresh_token, "refresh-token");
        assert_eq!(result.id_token, Some("id-token".into()));
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
