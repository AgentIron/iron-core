//! Validation for provider profile payloads before persistence.
//!
//! Provider profile records must contain only non-secret provider protocol
//! metadata. This module deserializes raw JSON into a [`ProviderProfile`]
//! and rejects payloads that contain credential secret material.

use crate::config::ConfigError;
use iron_providers::ProviderProfile;

/// Field names that indicate credential secret material.
///
/// If any of these appear as top-level keys in the JSON payload, the profile
/// is rejected. [`ProviderProfile`] itself does not have fields for these,
/// but callers may supply extra keys in the raw JSON that serde ignores.
///
/// Note: `credential_auth` is legitimate provider metadata (which credential
/// *kinds* are supported), not secret material, and is not forbidden.
const FORBIDDEN_SECRET_FIELDS: &[&str] = &[
    "api_key",
    "access_token",
    "refresh_token",
    "id_token",
    "password",
    "secret",
];

/// Deserialize and validate a provider profile JSON payload.
///
/// Returns the deserialized [`ProviderProfile`] on success, or a
/// [`ConfigError::Validation`] if the payload is malformed or contains
/// credential secret material.
pub fn validate_provider_profile(json: &str) -> Result<ProviderProfile, ConfigError> {
    // First, check for forbidden secret fields before deserialization.
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ConfigError::Deserialization(e.to_string()))?;

    if let serde_json::Value::Object(ref map) = value {
        for key in map.keys() {
            let lower = key.to_lowercase();
            if FORBIDDEN_SECRET_FIELDS.iter().any(|f| lower.contains(f)) {
                return Err(ConfigError::Validation(format!(
                    "Provider profile payload must not contain field '{}' (credential secret material is not allowed)",
                    key
                )));
            }
        }
    }

    // Deserialize into ProviderProfile to validate structure.
    let profile: ProviderProfile = serde_json::from_value(value).map_err(|e| {
        ConfigError::Deserialization(format!("Failed to deserialize provider profile: {}", e))
    })?;

    // Slug must be non-empty.
    if profile.slug.trim().is_empty() {
        return Err(ConfigError::Validation(
            "Provider profile slug must not be empty".to_string(),
        ));
    }

    // Base URL must be non-empty.
    if profile.base_url.trim().is_empty() {
        return Err(ConfigError::Validation(
            "Provider profile base_url must not be empty".to_string(),
        ));
    }

    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_providers::{ApiFamily, ProviderProfile};

    #[test]
    fn valid_profile_passes_validation() {
        let profile = ProviderProfile::new(
            "my-provider",
            ApiFamily::Completions,
            "https://api.example.com/v1",
        );
        let json = serde_json::to_string(&profile).unwrap();
        let validated = validate_provider_profile(&json).unwrap();
        assert_eq!(validated.slug, "my-provider");
    }

    #[test]
    fn profile_with_api_key_field_rejected() {
        let json = r#"{"slug":"test","family":"Completions","base_url":"https://example.com","credential_auth":[],"default_headers":{},"purpose":"General","quirks":{},"api_key":"sk-secret"}"#;
        let result = validate_provider_profile(json);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn profile_with_access_token_rejected() {
        let json = r#"{"slug":"test","family":"Completions","base_url":"https://example.com","credential_auth":[],"default_headers":{},"purpose":"General","quirks":{},"access_token":"tok-xyz"}"#;
        let result = validate_provider_profile(json);
        assert!(result.is_err());
    }

    #[test]
    fn malformed_json_rejected() {
        let result = validate_provider_profile("{not valid json}");
        assert!(result.is_err());
    }

    #[test]
    fn empty_slug_rejected() {
        let json = r#"{"slug":"","family":"Completions","base_url":"https://example.com","credential_auth":[],"default_headers":{},"purpose":"General","quirks":{}}"#;
        let result = validate_provider_profile(json);
        assert!(result.is_err());
    }

    #[test]
    fn empty_base_url_rejected() {
        let json = r#"{"slug":"test","family":"Completions","base_url":"","credential_auth":[],"default_headers":{},"purpose":"General","quirks":{}}"#;
        let result = validate_provider_profile(json);
        assert!(result.is_err());
    }

    #[test]
    fn profile_with_provider_guidance_passes() {
        let profile =
            ProviderProfile::new("custom", ApiFamily::Messages, "https://api.example.com")
                .with_provider_guidance("Use this provider carefully.");
        let json = serde_json::to_string(&profile).unwrap();
        let validated = validate_provider_profile(&json).unwrap();
        assert_eq!(
            validated.provider_guidance.as_deref(),
            Some("Use this provider carefully.")
        );
    }
}
