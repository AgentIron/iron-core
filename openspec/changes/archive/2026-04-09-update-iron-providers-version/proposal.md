## Why

`iron-core` still declares and documents `iron-providers = "0.1.0"` even though the latest compatible release is `0.1.1` and the crate already builds successfully against it. This change aligns the manifest and published guidance with the current supported dependency version without changing the existing Monty policy.

## What Changes

- Update the crate manifest to declare `iron-providers = "0.1.1"`.
- Update README and getting-started documentation to reference `iron-providers = "0.1.1"`.
- Clarify that `monty` remains a git dependency on `main` until a usable crates.io release exists.

## Capabilities

### New Capabilities
- `dependency-version-guidance`: Keeps published dependency declarations and setup guidance aligned with the versions `iron-core` currently supports.

### Modified Capabilities

## Impact

- Affected files: `Cargo.toml`, `README.md`, `docs/getting-started-iron-core.md`, and any nearby dependency-policy comments that reference Monty availability.
- No public API changes are intended.
- Dependency policy remains unchanged for `monty`; only `iron-providers` is bumped.
