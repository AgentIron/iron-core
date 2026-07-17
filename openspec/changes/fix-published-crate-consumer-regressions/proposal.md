## Why

The published `iron-core` 0.1.35 crate cannot link on Windows and cannot build with `embedded-python` after a fresh dependency resolution, even though the repository's locked Linux CI passes. The same report also exposes a facade API regression: `AgentSession::checkpoint()` is publicly available but can only return a placeholder error instead of entering the model-driven compaction flow.

## What Changes

- Compile only the scheduled-task host adapter for the current target while preserving platform-independent rendering tests on appropriate targets.
- Constrain the incompatible `get-size2` transitive release in the published dependency graph until the Monty/Ruff dependency chain can be upgraded coherently.
- Add package-consumer verification that links on Windows and resolves `embedded-python` without inheriting the repository lockfile.
- Make `AgentSession::checkpoint()` initiate the existing model-driven compression turn and report whether compaction completed successfully.
- Add regression coverage for target linking, fresh dependency resolution, and facade-level checkpoint behavior.

## Capabilities

### New Capabilities

- `published-crate-compatibility`: Defines cross-target linking and fresh-resolution checks from the perspective of a downstream crates.io consumer.

### Modified Capabilities

- `scheduled-automation-tasks`: Require platform-specific host adapter implementations to compile only on their supported targets.
- `dependency-version-guidance`: Require embedded-Python dependency constraints to preserve a coherent fresh-resolve graph and document temporary compatibility pins.
- `context-compaction`: Require facade checkpoint calls to invoke model-driven compaction rather than return an unconditional placeholder error.

## Impact

- Scheduled-task platform module declarations and target-specific adapter tests.
- `Cargo.toml` dependency constraints and the resolved `Cargo.lock`.
- Pull-request and release verification workflows, including a Windows linker job and a lockfile-independent consumer fixture or package smoke test.
- `AgentSession::checkpoint()`, the shared `/compact` prompt transformation, and context-management integration tests.
- Published crate behavior for Windows users and consumers enabling `embedded-python`; no intended breaking API change.
