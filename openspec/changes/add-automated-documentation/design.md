## Context

`iron-core` already enables rustdoc link lints and contains substantial crate-level narrative documentation, but strict generation with `-D warnings -D missing-docs` currently fails with 1,463 missing-documentation errors and six unresolved links. All 34 top-level public modules are considered intentional API for this change. Existing doctests report zero passing and fourteen ignored examples, while the README's installation, toolchain, dependency, release, and documentation guidance has drifted from the manifest and repository state.

The strict-rustdoc baseline consists primarily of 703 undocumented struct fields, 235 variants, 212 methods, 97 associated functions, 71 structs, 57 modules, 44 enums, 20 free functions, and 24 other public items. Implementation groups these diagnostics into crate/core tools, durable/context, runtime/prompt, persisted management, MCP/plugins, automation/scheduling, and transport/CLI domains. The baseline is documentation-only: no public visibility or signature reduction is permitted while clearing it.

The desired flow mirrors the documentation pipeline established in `AgentIron/iron-providers` PR #39: a dedicated local Invoke task, pull request validation, and rustdoc publication from `main`. This repository already has GitHub Pages configured as a workflow deployment at the `core.agentiron.ai` custom domain.

## Goals / Non-Goals

**Goals:**

- Give every currently public module and item meaningful inline rustdoc.
- Provide compiling examples for primary public usage paths rather than examples for every trivial accessor or field.
- Compile README Rust snippets as doctests and comprehensively review the README's factual claims.
- Make documentation warnings, broken links, missing public docs, and example drift fail locally and in pull request CI.
- Publish current `main` rustdoc through the existing GitHub Pages configuration with least-privilege workflow permissions.
- Keep generated documentation conventional and compatible with future docs.rs publication.

**Non-Goals:**

- Do not reduce module or item visibility, change public signatures, or otherwise redesign the public API.
- Do not change runtime behavior or dependencies solely for documentation convenience.
- Do not add missing-documentation exceptions for existing public surfaces.
- Do not add a static-site generator, custom documentation theme, or pull request preview deployments.
- Do not require an independent example for every field, variant, or trivial accessor.
- Do not manage the custom domain, DNS, certificate, or HTTPS-enforcement setting in repository code.

## Decisions

### Treat Rust visibility as the documentation boundary

Every item reachable as public API during an all-features documentation build will be documented. Public modules that appear implementation-oriented remain public and documented in this change so the generated site can support a later API review.

Alternative considered: narrow visibility or allow missing docs on legacy modules. This would reduce the immediate workload, but it would combine an API compatibility change with documentation work and prevent a complete review of the currently exposed surface.

### Use rustdoc as the publication artifact

The Pages build will publish `target/doc` from `cargo doc --manifest-path Cargo.toml --no-deps --all-features`. A generated root `index.html` will redirect to `iron_core/index.html`, making the custom domain useful without a separate landing site.

Alternative considered: introduce a static-site generator. Rustdoc already provides item navigation, search, links, and examples, while a second documentation stack would create avoidable maintenance and drift.

### Enforce documentation through a dedicated Invoke task

`inv docs` will run these two checks as a separate documentation group:

```bash
RUSTDOCFLAGS="-D warnings -D missing-docs" cargo doc --manifest-path Cargo.toml --no-deps --all-features
cargo test --manifest-path Cargo.toml --doc
```

Keeping documentation separate from `inv build` makes failures locally identifiable and preserves the existing build task's purpose. Pull request CI will invoke the same task so local and hosted validation share one command definition.

Alternative considered: duplicate raw Cargo commands in CI or fold them into `inv build`. Duplication invites command drift, while combining the tasks obscures whether a failure comes from compilation, linting, rustdoc, or doctests.

### Prefer useful rustdoc over lint-silencing text

Documentation will describe semantics, invariants, side effects, ownership, errors, and lifecycle constraints where relevant. Module docs will explain purpose and relationship to neighboring modules. Repetitive fields and variants may use concise descriptions, but placeholder wording that merely restates an identifier is not sufficient.

Usage-path examples will focus on recommended integrations: facade sessions and streaming, configuration, tool registration, providers and credentials, context management, profiles and stored prompts, management APIs, transports, MCP, plugins, skills, headless automation, scheduled tasks, and debugging. Existing ignored examples will be converted to compiling examples where practical.

### Treat README review and README doctests as complementary checks

The README will be included from `src/lib.rs` through `#[doc = include_str!("../README.md")]` on a `#[cfg(doctest)]` item. Rust snippets will compile normally or use `no_run` when executing them would require credentials, network access, filesystem state, platform services, or a long-running process. Hidden scaffolding will keep rendered examples readable. `ignore` will be reserved for examples that cannot reasonably compile in the documentation environment.

Doctests cannot validate prose, TOML, shell commands, links, or operational claims, so the README will also receive a manual claim-by-claim review against `Cargo.toml`, current public APIs, workflows, releases, crates.io, and files in the repository. Installation guidance will reflect that `iron-core` is published, the minimum Rust version will match the manifest, release guidance will match current workflows, and broken documentation links will be removed or repaired.

Alternative considered: only make the existing Rust snippet compile. That would leave known false statements in the primary project entry point and fail the stated goal of current consumer documentation.

### Separate pull request validation from Pages publication

Pull requests to `main` will run `inv docs` in the existing Rust checks job and will not publish. A dedicated Pages workflow will build on pushes to `main` and manual dispatch, upload `target/doc`, and deploy it in a separate job.

The workflow will set only `contents: read` globally. The build checkout will use `persist-credentials: false`. Only the deploy job will receive `pages: write` and `id-token: write`, and deployment concurrency will cancel superseded Pages runs.

Alternative considered: deploy previews for pull requests. Preview lifecycle and permission complexity are unnecessary for the initial capability.

## Risks / Trade-offs

- [The documentation sweep is large and may produce superficial comments] -> Review by domain, require semantic descriptions, and validate the rendered output rather than treating lint success as completion.
- [Documenting currently public internals may appear to strengthen their compatibility promise] -> State that all current public surfaces are documented for review while deferring any visibility decision to a separate breaking-change assessment.
- [All-features documentation increases build time and exercises optional embedded-Python dependencies] -> Cache Rust artifacts in CI and retain all-features coverage because published docs must represent the complete public surface.
- [Doctest examples can accidentally perform external side effects] -> Use `no_run` and minimal hidden setup while still requiring compilation.
- [README facts can drift even when doctests pass] -> Include a structured manual README audit in the implementation and review tasks.
- [Pages deployment depends on repository-level settings] -> Use the already configured workflow-based Pages environment and document the custom-domain setup; keep validation independent so docs remain buildable if deployment is temporarily disabled.
- [The custom domain currently does not enforce HTTPS] -> Record HTTPS enforcement as an operational follow-up rather than expanding this repository change into DNS or Pages administration.

## Migration Plan

1. Add the local documentation task so progress can be measured consistently.
2. Repair existing rustdoc link failures and document public modules and items by domain until strict generation succeeds.
3. Add and convert usage-path examples, then enable README doctest inclusion.
4. Audit and update README content against current repository and release state.
5. Add pull request validation and the Pages deployment workflow.
6. Run all existing build, test, documentation, security, and package-consumer checks.
7. Review the generated site locally before merging; after merge, verify the Pages deployment and custom-domain redirect.

Rollback of publication consists of disabling or removing the Pages workflow. Local and pull request documentation validation can remain active independently. Rustdoc comments and README corrections require no data migration.

## Open Questions

- Should HTTPS enforcement for `core.agentiron.ai` be enabled immediately as a repository-settings follow-up, or tracked separately after the first successful deployment?
- Which currently public modules should become candidates for a later visibility and stability review once the complete generated API can be inspected?

## Follow-up Work

- After the first successful Pages deployment, enable HTTPS enforcement for
  `core.agentiron.ai` in repository settings and verify HTTP redirects to HTTPS.
- Review the published API before the next breaking release. Initial visibility
  candidates include orchestration internals (`prompt_runner`,
  `prompt_lifecycle`, and `request_builder`), low-level persistence modules
  (`config::crypto`, `config::migrations`, and `config::records`), built-in tool
  helpers/renderers, MCP protocol/client internals, and plugin lifecycle and
  effective-tool internals. Any visibility reduction requires a separate
  compatibility assessment and change proposal.
