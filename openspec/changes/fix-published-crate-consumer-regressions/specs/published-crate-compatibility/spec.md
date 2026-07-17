## ADDED Requirements

### Requirement: Published crate SHALL link on supported targets
The packaged `iron-core` crate SHALL compile and link on Linux, macOS, and Windows without referencing symbols unavailable on the selected target.

#### Scenario: Windows consumer links iron-core
- **WHEN** CI builds a downstream consumer of the packaged crate on a native Windows runner
- **THEN** the consumer links successfully
- **AND** no macOS or Unix-only scheduler symbol is included in the Windows artifact

#### Scenario: Supported platform verification exercises the linker
- **WHEN** CI verifies a supported native target
- **THEN** it runs a build or test command that performs linking rather than relying only on `cargo check`

### Requirement: Published features SHALL build from a fresh resolution
The packaged `iron-core` crate SHALL build with each supported public feature combination when resolved without the repository's `Cargo.lock`.

#### Scenario: Fresh embedded-Python consumer resolves dependencies
- **WHEN** CI creates a new downstream crate with no existing lockfile and enables `iron-core/embedded-python`
- **THEN** Cargo resolves one compatible dependency graph
- **AND** the downstream crate builds successfully

#### Scenario: Consumer verification uses packaged metadata
- **WHEN** CI verifies lockfile-independent resolution
- **THEN** it consumes the package contents or an equivalent downstream path dependency from outside the repository workspace
- **AND** it does not copy or inherit the repository `Cargo.lock`
