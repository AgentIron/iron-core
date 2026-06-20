## ADDED Requirements

### Requirement: Provider credential support SHALL use effective provider profile metadata
Provider credential orchestration SHALL derive API-key, OAuth bearer, and no-auth support from the effective provider profile metadata when a profile is available for the provider slug.

Provider-specific OAuth device-code metadata MAY remain specialized, but generic credential-mode support used for provider invocation SHALL not rely solely on hardcoded built-in slug lists.

#### Scenario: Custom profile supports API-key credentials
- **WHEN** credential orchestration resolves credentials for a custom provider profile that supports API-key authentication
- **THEN** API-key credentials are accepted for that provider
- **AND** provider invocation uses the auth strategy declared by the effective provider profile

#### Scenario: Effective profile supports OAuth bearer credentials
- **WHEN** credential orchestration resolves credentials for an effective provider profile that supports OAuth bearer authentication
- **THEN** stored OAuth bearer credentials are considered usable for that provider
- **AND** unsupported credential modes are rejected with an actionable unsupported-credential error

#### Scenario: Effective profile supports no-auth invocation
- **WHEN** credential orchestration resolves credentials for an effective provider profile that supports no-auth invocation
- **THEN** the provider can be constructed without requiring API-key or OAuth credential material

#### Scenario: No effective profile metadata is available
- **WHEN** credential orchestration cannot find effective provider profile metadata for a provider slug
- **THEN** credential resolution returns an actionable unsupported-provider or unsupported-credential outcome
- **AND** it does not silently assume all credential modes are supported
