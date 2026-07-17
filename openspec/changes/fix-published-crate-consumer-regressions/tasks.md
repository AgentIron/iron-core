## 1. Platform-Correct Scheduler Compilation

- [x] 1.1 Add target configuration to the scheduled-task platform module declarations for Linux cron/crontab, macOS launchd, and Windows Task Scheduler
- [x] 1.2 Update any imports or tests that assume foreign platform adapter modules are available on the current target
- [x] 1.3 Verify the Linux scheduler factory and adapter tests still compile and pass after module gating
- [x] 1.4 Add native Windows linking coverage that builds `iron-core` with the supported feature set and catches unresolved platform symbols
- [x] 1.5 Add or confirm native macOS coverage for launchd compilation and its colocated rendering tests

## 2. Fresh Embedded-Python Resolution

- [x] 2.1 Add an optional exact `get-size2` 0.10.1 compatibility dependency to `Cargo.toml` and enable it only through `embedded-python`
- [x] 2.2 Document the upstream Ruff/`compact_str` incompatibility and the condition for removing the temporary constraint
- [x] 2.3 Regenerate `Cargo.lock` and verify the embedded-Python graph contains the compatible `get-size2` and `compact_str` versions
- [x] 2.4 Add an external consumer smoke test with no inherited lockfile that enables `embedded-python` and builds against the package contents or equivalent external path dependency

## 3. Facade Checkpoint Compaction

- [x] 3.1 Route `AgentSession::checkpoint()` through the error-preserving `/compact` prompt path after its existing idle and context-management precondition checks
- [x] 3.2 Replace the placeholder-error regression test with a mock-provider flow that invokes `compress` and verifies durable compressed context is produced
- [x] 3.3 Add checkpoint tests for prompt execution failure propagation and preserve tests for disabled context management and active-session rejection
- [x] 3.4 Verify direct facade `prompt("/compact")` and `checkpoint()` receive the same compression-focused instruction without persisting the literal command

## 4. Consumer CI And Release Gates

- [x] 4.1 Integrate the native Windows linker check into pull-request CI using the pinned Rust toolchain
- [x] 4.2 Integrate the lockfile-independent embedded-Python consumer smoke test into pull-request CI
- [x] 4.3 Run the same consumer smoke checks before publication in patch and manual release workflows without replacing the existing locked reproducibility checks
- [x] 4.4 Keep duplicated workflow commands synchronized through a shared script or an explicitly identical command sequence

## 5. Verification

- [x] 5.1 Run `cargo fmt --manifest-path Cargo.toml -- --check`
- [x] 5.2 Run `cargo clippy --locked --manifest-path Cargo.toml --all-targets --all-features -- -D warnings`
- [x] 5.3 Run `cargo test --locked --manifest-path Cargo.toml` and the targeted context-management tests
- [x] 5.4 Run the fresh external `embedded-python` consumer build without a preexisting lockfile
- [x] 5.5 Confirm native Windows CI performs a successful link and native macOS CI compiles the launchd adapter
- [x] 5.6 Run `openspec validate fix-published-crate-consumer-regressions --strict`
