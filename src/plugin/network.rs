use serde::{Deserialize, Serialize};

/// Network access policy declared by a plugin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// Allow only specific domains
    Allowlist(Vec<String>),
    /// Block specific domains (allow everything else)
    Blocklist(Vec<String>),
    /// Allow any outbound HTTP/HTTPS access
    Wildcard,
}

impl NetworkPolicy {
    /// Check if a domain is allowed by this policy
    pub fn allows(&self, domain: &str) -> bool {
        match self {
            Self::Allowlist(domains) => {
                domains.iter().any(|allowed| {
                    // Exact match
                    domain == allowed
                    // Subdomain match (e.g., api.example.com matches example.com)
                    || domain.ends_with(&format!(".{}", allowed))
                })
            }
            Self::Blocklist(domains) => !domains
                .iter()
                .any(|blocked| domain == blocked || domain.ends_with(&format!(".{}", blocked))),
            Self::Wildcard => true,
        }
    }

    /// Get a summary description of the policy
    pub fn summary(&self) -> String {
        match self {
            Self::Allowlist(domains) => {
                if domains.is_empty() {
                    "No network access".to_string()
                } else if domains.len() == 1 {
                    format!("Access to {}", domains[0])
                } else {
                    format!("Access to {} domains", domains.len())
                }
            }
            Self::Blocklist(domains) => {
                if domains.is_empty() {
                    "Full network access".to_string()
                } else {
                    format!("Full network access except {} domains", domains.len())
                }
            }
            Self::Wildcard => "Full network access".to_string(),
        }
    }
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self::Allowlist(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_exact_match() {
        let policy = NetworkPolicy::Allowlist(vec!["api.example.com".to_string()]);
        assert!(policy.allows("api.example.com"));
        assert!(!policy.allows("other.example.com"));
    }

    #[test]
    fn test_allowlist_subdomain_match() {
        let policy = NetworkPolicy::Allowlist(vec!["example.com".to_string()]);
        assert!(policy.allows("example.com"));
        assert!(policy.allows("api.example.com"));
        assert!(policy.allows("v1.api.example.com"));
        assert!(!policy.allows("other.com"));
    }

    #[test]
    fn test_blocklist() {
        let policy = NetworkPolicy::Blocklist(vec!["blocked.com".to_string()]);
        assert!(!policy.allows("blocked.com"));
        assert!(!policy.allows("api.blocked.com"));
        assert!(policy.allows("allowed.com"));
    }

    #[test]
    fn test_wildcard() {
        let policy = NetworkPolicy::Wildcard;
        assert!(policy.allows("any.domain.com"));
        assert!(policy.allows("localhost"));
    }

    #[test]
    fn test_summary() {
        let policy = NetworkPolicy::Allowlist(vec!["api.example.com".to_string()]);
        assert_eq!(policy.summary(), "Access to api.example.com");

        let policy = NetworkPolicy::Allowlist(vec!["a.com".to_string(), "b.com".to_string()]);
        assert_eq!(policy.summary(), "Access to 2 domains");

        let policy = NetworkPolicy::Wildcard;
        assert_eq!(policy.summary(), "Full network access");
    }
}
