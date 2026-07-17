//! Redacting wrappers for secret-bearing strings.
//!
//! Types in this module keep their value reachable for serialization and
//! runtime use while preventing accidental disclosure through
//! [`Debug`](std::fmt::Debug) formatting. They are used for durable and
//! handoff fields that carry provider credentials.
//!
//! # Serialization
//!
//! [`SecretString`] serializes transparently as a plain JSON string, so
//! existing serialized sessions and handoff bundles remain compatible and
//! credential portability is preserved. The redaction only affects debug
//! output, not persistence.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A string wrapper that serializes transparently but redacts its value in
/// [`Debug`](fmt::Debug) output.
///
/// Use this for fields that must round-trip through serialized payloads (for
/// example durable sessions and handoff bundles) but should never appear in
/// logs or debug-rendered state. The wrapped value is still serialized as a
/// plain string, preserving compatibility with previously persisted data.
///
/// # Examples
///
/// ```
/// use iron_core::SecretString;
///
/// let key = SecretString::new("sk-example".to_string());
/// assert_eq!(key.reveal(), "sk-example");
/// assert_eq!(format!("{key:?}"), "SecretString(<redacted>)");
/// ```
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// Wraps a secret string.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Returns the secret value.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}
