## ADDED Requirements

### Requirement: Embedded Python dependency graph SHALL remain resolvable
The project SHALL constrain known-incompatible transitive releases required by `embedded-python` until the owning direct dependency can be upgraded to a compatible graph. Compatibility constraints SHALL be included in the published manifest rather than relying on the repository lockfile.

#### Scenario: Incompatible get-size2 patch is excluded
- **WHEN** a consumer freshly resolves `iron-core` with `embedded-python`
- **THEN** Cargo does not select a `get-size2` release whose `compact_str` type is incompatible with the Ruff dependency graph
- **AND** `ruff_python_ast` derives its size implementation successfully

#### Scenario: Temporary compatibility constraint is documented
- **WHEN** a maintainer inspects the embedded-Python dependencies in `Cargo.toml`
- **THEN** the manifest identifies the upstream incompatibility guarded by the constraint
- **AND** states the condition under which the constraint can be removed
