## ADDED Requirements

### Requirement: Host adapter compilation SHALL follow the target platform
Platform-specific host scheduler implementation modules SHALL compile only for their supported target operating system. Target-independent scheduling contracts MAY remain available on every target.

#### Scenario: Windows excludes launchd implementation
- **WHEN** `iron-core` is compiled for Windows
- **THEN** the macOS launchd implementation is excluded from the artifact
- **AND** no reference to the Unix `getuid` symbol is emitted

#### Scenario: Platform factory resolves an available module
- **WHEN** the host scheduler factory is compiled for Linux, macOS, or Windows
- **THEN** the adapter selected by its target-specific branch is available for that target
- **AND** adapters for other targets are not required to compile or link
