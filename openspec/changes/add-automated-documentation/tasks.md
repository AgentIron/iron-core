## 1. Documentation Validation Foundation

- [x] 1.1 Add and register an `inv docs` task that runs strict all-features rustdoc generation and crate doctests using the commands required by the specification
- [x] 1.2 Repair the existing unresolved rustdoc links so link warnings no longer obscure missing-documentation progress
- [x] 1.3 Record a domain-grouped baseline of strict rustdoc diagnostics to guide the documentation sweep and confirm that no public visibility is reduced during cleanup

## 2. Core Public API Rustdoc

- [x] 2.1 Document crate-level modules, public re-exports, the prelude, and public binary-facing items with purpose and navigation context
- [x] 2.2 Document configuration, errors, schema validation, capabilities, tools, and built-in-tool public APIs, including fields, variants, invariants, and failure behavior
- [x] 2.3 Document durable and ephemeral session state, messages, timeline records, tool-call records, and their lifecycle semantics
- [x] 2.4 Document context accounting, compaction, handoff, model switching, and context-policy public APIs
- [x] 2.5 Document the facade, connections, runtime, prompt turns, prompt lifecycle, prompt runner, prompt composition, and request-building public APIs
- [x] 2.6 Document profile, provider-profile, provider-credential, stored-prompt, management, and config-store public APIs
- [x] 2.7 Document MCP and integration-plugin public APIs, including lifecycle, authentication, availability, execution, network, and rich-output contracts
- [x] 2.8 Document automation-task, scheduled-task, headless execution, delegation, skills, and execution-resolution public APIs
- [x] 2.9 Document transport, CLI support, embedded Python, and debug-observation public APIs
- [x] 2.10 Run strict rustdoc after each domain pass and resolve every remaining missing module, type, field, variant, trait item, method, function, constant, macro, and warning without adding missing-docs exceptions

## 3. Compiling Usage Examples

- [x] 3.1 Add compiling examples for the recommended `IronAgent` connection, session, streaming prompt, approval, cancellation, and tool-registration paths
- [x] 3.2 Add compiling examples for configuration, providers and credentials, profiles and stored prompts, context management, and management APIs
- [x] 3.3 Add compiling examples for MCP, plugins, skills, transports, headless automation, scheduled tasks, and debug observation where they improve discovery of primary usage paths
- [x] 3.4 Review all existing ignored rustdoc examples and convert each practical example to a normal or `no_run` doctest with minimal hidden scaffolding
- [x] 3.5 Document a concrete reason for any example that must remain ignored and verify that no example performs external side effects during doctests

## 4. README Review And Doctests

- [x] 4.1 Audit every README factual claim against `Cargo.toml`, current public APIs, repository files, workflows, GitHub releases, crates.io, and the configured Pages custom domain
- [x] 4.2 Update installation, dependency, feature, minimum-Rust-version, and public-API-positioning guidance for the published crate and current supported API
- [x] 4.3 Update the quick-start example to current APIs and dependencies, preserve reader-friendly rendering, and make it compile as a normal or `no_run` doctest
- [x] 4.4 Update built-in tool, MCP, plugin, and other feature descriptions to match current implementation and avoid unsupported completeness or stability claims
- [x] 4.5 Update development, CI, release, and crates.io publication guidance so documented commands and behavior match the current workflows
- [x] 4.6 Repair or remove stale documentation links and add the published API documentation URL and existing GitHub Pages setup details
- [x] 4.7 Include README content from `src/lib.rs` with `#[doc = include_str!("../README.md")]` on a `#[cfg(doctest)]` item and verify README API drift fails doctests

## 5. CI And GitHub Pages

- [x] 5.1 Add `inv docs` to the pull request Rust checks so documentation failures block pull requests without deploying artifacts
- [x] 5.2 Add a Pages workflow triggered by pushes to `main` and manual dispatch that builds strict all-features rustdoc and writes a root redirect to `iron_core/index.html`
- [x] 5.3 Upload `target/doc` with the official Pages artifact action and deploy it from a separate job using the `github-pages` environment
- [x] 5.4 Scope workflow-level permissions to `contents: read`, disable checkout credential persistence in the build job, and grant `pages: write` plus `id-token: write` only to the deploy job
- [x] 5.5 Configure Pages workflow concurrency to cancel superseded deployments and confirm a failed build cannot reach the deploy job

## 6. End-To-End Verification And Review

- [x] 6.1 Run `inv docs` and confirm strict rustdoc succeeds with no missing public documentation, warnings, broken links, or failing doctests
- [x] 6.2 Run `inv build`, `inv test`, `inv security`, and the fresh package-consumer check to confirm documentation changes do not alter runtime or package behavior
- [x] 6.3 Inspect the locally generated rustdoc site for navigation, module descriptions, terminology, cross-links, and discoverable examples across every public domain
- [x] 6.4 Perform a separate rendered README review covering prose, TOML, shell commands, links, installation, releases, and Pages guidance beyond what doctests validate
- [x] 6.5 Review the final diff for accidental public visibility, signature, behavior, dependency, or generated-file changes
- [ ] 6.6 After merge, verify the Pages workflow succeeds and both `core.agentiron.ai` and the `iron_core` crate page resolve correctly
- [x] 6.7 Record HTTPS enforcement for `core.agentiron.ai` and any public-surface reduction candidates as explicit operational or API-review follow-ups
