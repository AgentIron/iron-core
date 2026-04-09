## ADDED Requirements

### Requirement: Provider dependency version guidance stays current
The project SHALL declare and document the current supported `iron-providers` version consistently across the crate manifest and user-facing setup guidance.

#### Scenario: Manifest and docs reference the same provider version
- **WHEN** a maintainer reviews the crate manifest and setup documentation
- **THEN** `Cargo.toml`, `README.md`, and getting-started guidance reference the same supported `iron-providers` version

### Requirement: Monty source policy remains explicit
The project SHALL document that `monty` remains sourced from git until a usable crates.io library release is available.

#### Scenario: Embedded Python guidance explains Monty source choice
- **WHEN** a maintainer or integrator reads the dependency policy comments and setup documentation
- **THEN** they can see that `monty` is intentionally kept on git `main` pending a usable crates.io release
