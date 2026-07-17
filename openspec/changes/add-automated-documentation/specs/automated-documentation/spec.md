## ADDED Requirements

### Requirement: Complete public API documentation
The crate SHALL provide meaningful inline rustdoc for every module and API item that is public in an all-features documentation build, including public structs, enums, traits, variants, fields, constructors, builders, methods, functions, constants, and macros. This change SHALL NOT narrow existing public visibility or exempt existing public surfaces from missing-documentation validation.

#### Scenario: Strict public documentation coverage
- **WHEN** rustdoc builds the crate with all features and missing documentation denied
- **THEN** every currently public module and item has sufficient inline documentation
- **AND** the build completes without a missing-documentation diagnostic

#### Scenario: Existing public surface is preserved
- **WHEN** documentation is added to an implementation-oriented public module or item
- **THEN** its visibility and public signature remain unchanged

### Requirement: Strict local documentation validation
The repository SHALL provide an `inv docs` task that runs strict rustdoc generation and documentation tests using the root manifest.

#### Scenario: Local documentation task succeeds
- **WHEN** a contributor runs `inv docs`
- **THEN** it runs `RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --manifest-path Cargo.toml --no-deps --all-features`
- **AND** it runs `cargo test --manifest-path Cargo.toml --doc`
- **AND** it reports success only when both commands succeed

#### Scenario: Rustdoc warning fails validation
- **WHEN** public documentation contains a broken link, another rustdoc warning, or an undocumented public item
- **THEN** `inv docs` exits unsuccessfully

#### Scenario: Doctest drift fails validation
- **WHEN** a compiled documentation example no longer matches the public API
- **THEN** `inv docs` exits unsuccessfully

### Requirement: Compiling public usage examples
Primary public usage paths SHALL have compiling rustdoc examples. Examples SHALL compile without performing network, credential, filesystem, platform-service, or long-running side effects during documentation tests.

#### Scenario: Side-effect-free example
- **WHEN** an example can execute deterministically without external state
- **THEN** it is compiled and run as a normal doctest

#### Scenario: Side-effecting example
- **WHEN** an example requires external credentials, network access, filesystem state, platform services, or a long-running process
- **THEN** it uses `no_run` and any minimal hidden scaffolding needed to verify compilation without executing the side effect

#### Scenario: Existing ignored examples are reviewed
- **WHEN** an existing public example is marked `ignore`
- **THEN** it is converted to a compiling normal or `no_run` doctest where practical
- **AND** any remaining ignored example has a concrete reason that prevents compilation in the documentation environment

### Requirement: README examples participate in doctests
The crate SHALL include README content in rustdoc tests through `#[doc = include_str!("../README.md")]` on an item gated by `#[cfg(doctest)]`. README Rust examples SHALL compile against the current public API unless they have a documented reason not to.

#### Scenario: README API drift
- **WHEN** a README Rust snippet references a removed or changed API
- **THEN** `cargo test --manifest-path Cargo.toml --doc` fails

#### Scenario: README example has runtime side effects
- **WHEN** a README example would contact a provider or otherwise depend on external state
- **THEN** the rendered example remains useful to readers
- **AND** the example uses `no_run` or hidden scaffolding so its code is still compiled

### Requirement: README factual accuracy
The README SHALL be reviewed and updated against the current manifest, public API, repository files, workflows, GitHub release state, crates.io publication state, and GitHub Pages configuration. The review SHALL cover installation, minimum Rust version, dependency guidance, public API positioning, examples, feature descriptions, development commands, release and publication behavior, documentation links, and API documentation access.

#### Scenario: Published crate installation guidance
- **WHEN** a reader follows the primary installation instructions
- **THEN** the instructions identify the currently supported crates.io dependency approach
- **AND** dependency and feature examples are compatible with the current manifest

#### Scenario: Toolchain and workflow guidance
- **WHEN** a reader follows the development instructions
- **THEN** the documented minimum Rust version agrees with `Cargo.toml`
- **AND** descriptions of CI, releases, and publication agree with the repository workflows and current release process

#### Scenario: README documentation links
- **WHEN** a reader follows a repository documentation link
- **THEN** the target exists and is relevant

#### Scenario: Public API positioning
- **WHEN** the README recommends the facade and runtime APIs as primary entry points
- **THEN** it does not incorrectly state that other currently public modules are unsupported or outside the public API

### Requirement: Pull request documentation gate
Pull requests targeting `main` SHALL run `inv docs` as a required documentation validation step and SHALL NOT deploy documentation.

#### Scenario: Pull request documentation succeeds
- **WHEN** a pull request has complete public docs, warning-free rustdoc, and passing doctests
- **THEN** its documentation validation step succeeds

#### Scenario: Pull request documentation fails
- **WHEN** a pull request introduces an undocumented public item, rustdoc warning, broken documentation link, or failing doctest
- **THEN** its documentation validation step fails
- **AND** no Pages deployment is attempted

### Requirement: Main-branch GitHub Pages publication
Pushes to `main` and manually dispatched documentation workflows SHALL build all-features rustdoc without dependencies, publish `target/doc` through GitHub Pages, and make the crate documentation reachable through the repository's existing custom domain.

#### Scenario: Main branch documentation deployment
- **WHEN** a commit is pushed to `main` and strict rustdoc generation succeeds
- **THEN** `target/doc` is uploaded as a Pages artifact
- **AND** a separate deploy job publishes the artifact

#### Scenario: Documentation root navigation
- **WHEN** a reader opens the documentation site's root URL
- **THEN** the site redirects to `iron_core/index.html`

#### Scenario: Failed documentation build
- **WHEN** strict rustdoc generation fails on `main`
- **THEN** the deploy job does not publish a new Pages artifact

### Requirement: Least-privilege Pages workflow
The Pages workflow SHALL use least-privilege permissions and SHALL prevent checkout credentials from persisting in the documentation build job.

#### Scenario: Build job permissions
- **WHEN** the documentation build job checks out the repository
- **THEN** workflow-level permissions grant only `contents: read`
- **AND** checkout uses `persist-credentials: false`

#### Scenario: Deploy job permissions
- **WHEN** the Pages deploy job runs
- **THEN** `pages: write` and `id-token: write` are scoped to that job
- **AND** those write permissions are not granted to the build job

### Requirement: Generated documentation review
Completion SHALL include a review of the rendered rustdoc site for navigation, module descriptions, terminology, examples, and cross-links, in addition to automated validation.

#### Scenario: Pre-merge generated-site review
- **WHEN** strict documentation checks pass
- **THEN** a reviewer can navigate the generated `iron_core` documentation locally
- **AND** primary public domains have understandable module-level descriptions and discoverable usage examples

#### Scenario: Post-merge publication verification
- **WHEN** the change merges to `main`
- **THEN** the Pages workflow succeeds
- **AND** the custom-domain root and crate documentation page are reachable
