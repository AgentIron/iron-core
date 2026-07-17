## Context

Issue 102 identifies three consumer-visible regressions in 0.1.35. `scheduled_task::platform` declares all adapters on every target even though the factory selects them with `cfg_if!`; consequently the Windows archive contains launchd's unresolved `getuid` reference. The committed lockfile selects `get-size2` 0.10.1, but the published manifest permits 0.10.2, whose patch-level `compact_str` change is incompatible with Monty's Ruff graph. Finally, the facade's `checkpoint()` preserves old precondition checks but always returns a placeholder error, while `/compact` already reaches model-driven compression through `IronConnection::handle_prompt`.

The current pull-request and release workflows build only on Ubuntu and use `--locked`. This validates the repository state but not the target linker or dependency graph seen by a fresh downstream consumer.

## Goals / Non-Goals

**Goals:**

- Ensure each supported target compiles and links only its native scheduler implementation.
- Make the published `embedded-python` feature resolve to the known-compatible Monty/Ruff graph without a producer lockfile.
- Verify package consumption through native linking and lockfile-independent dependency resolution.
- Restore `AgentSession::checkpoint()` as a facade entry point to the existing model-driven compaction turn.
- Keep each fix independently releasable and independently covered by regression tests.

**Non-Goals:**

- Redesign host scheduler adapters or their public contract.
- Fork or patch Monty, Ruff, `get-size2`, or `compact_str` source code.
- Permanently own transitive version selection after upstream dependencies become compatible.
- Reintroduce hidden summarizer-based compaction or guarantee that a model chooses a valid compression range.
- Change compaction thresholds, compression range validation, or lifecycle event schemas.

## Decisions

### Gate adapter modules at their platform boundary

Apply target configuration to the module declarations: Linux owns `cron_adapter` and `crontab`, macOS owns `launchd`, and Windows owns `task_scheduler`. This aligns compilation with the already target-gated factory and prevents unsupported foreign-function or operating-system assumptions from entering an artifact.

```text
                 scheduled_task::host factory
                            |
          +-----------------+-----------------+
          |                 |                 |
       Linux              macOS             Windows
          |                 |                 |
   cron_adapter          launchd       task_scheduler
      crontab             getuid         schtasks.exe
```

Platform-independent adapter rendering tests remain colocated and run on their native CI target. If future work needs every renderer test on every host, pure conversion/rendering code can be extracted into target-neutral modules; that extraction is not required to fix the linker defect.

Alternative considered: gate only `extern_uid()`. Rejected because it leaves unsupported adapters compiled on every target and perpetuates the mismatch between module documentation and actual boundaries.

### Add a temporary direct compatibility constraint

Declare `get-size2` as an exact, optional compatibility dependency enabled by `embedded-python`, pinned to the known-compatible 0.10.1 release. A direct published constraint participates in downstream resolution; the producer's `Cargo.lock` does not. The manifest comment records the Ruff/`compact_str` mismatch and the removal condition: upgrade Monty/Ruff to versions compatible with the newer `get-size2` graph, then remove the pin after fresh-resolution verification passes.

Alternative considered: use only `<0.10.2`. This expresses the incompatibility more semantically, but an exact known-good patch minimizes exposure to another semver-breaking 0.10.x release while this is a temporary transitive guard. Alternative considered: patch crates.io source. Rejected because patches do not belong in a reusable library manifest for this case and would create source ownership overhead.

### Test from the downstream consumer boundary

CI will have two complementary checks:

- A native Windows job runs a linking command against all relevant targets/features, catching unresolved symbols that `cargo check` cannot detect.
- A fresh temporary consumer outside the repository workspace depends on the package/path with `embedded-python`, starts without a lockfile, and runs a build. This catches incompatible transitive updates hidden by the repository lockfile.

Release workflows retain locked reproducibility checks, but package-consumer smoke verification runs before publication. The same smoke procedure should be shared or kept structurally identical across pull-request and release workflows so release behavior cannot drift.

Alternative considered: run `cargo update` in the repository. Rejected because it mutates the producer lockfile and still does not accurately model a downstream package consumer. Alternative considered: use cross-compilation from Ubuntu for Windows. Rejected as the sole check because installing a cross linker is more fragile and less representative than a native runner.

### Route checkpoint through the existing compact command path

After preserving its idle and context-management preconditions, `AgentSession::checkpoint()` invokes the facade's error-preserving prompt path with `/compact`. `try_prompt()` already calls `IronConnection::handle_prompt`, where `/compact` is replaced by the compression-focused instruction before the prompt is added to durable history. This gives ACP and facade callers one behavior rather than duplicating prompt-runner setup or compression instructions.

```text
AgentSession::checkpoint()
          |
          v
 AgentSession::try_prompt("/compact")
          |
          v
 IronConnection::handle_prompt
          |
          +--> replace command with compaction instruction
          +--> run model turn with built-in compress tool
```

`checkpoint()` returns `Ok(())` when the compression-focused prompt turn completes and propagates connection-level prompt failures. It does not claim that the model necessarily selected a useful valid range; durable compressed-block changes and lifecycle events remain the authoritative evidence of actual compaction.

Alternative considered: duplicate the prompt runner and invoke `compress` directly. Rejected because summaries and ranges are model-authored, and a second orchestration path would drift from `/compact` behavior.

## Risks / Trade-offs

- [Gating whole adapter modules reduces cross-host unit-test coverage] -> Run adapter tests on native target jobs and defer pure-renderer extraction unless CI cost or platform availability requires it.
- [An exact transitive pin can block a compatible patch] -> Document it as temporary and remove it only with an upstream Monty/Ruff upgrade plus fresh-resolution coverage.
- [Native Windows CI increases latency and runner cost] -> Keep the job focused on linking and targeted tests rather than duplicating every Linux quality check.
- [Fresh resolution can fail because of unrelated registry changes] -> Treat this as intentional early warning for a published library; preserve locked checks for deterministic diagnosis.
- [A model-driven checkpoint turn may complete without compacting] -> Define success as successful execution of the requested turn and verify durable compaction separately in integration tests where the mock model calls `compress`.
- [Idle precheck races with another prompt] -> Rely on the existing runtime prompt-start guard as the final authority and propagate its failure.

## Migration Plan

1. Add target gates and verify native Linux, macOS, and Windows compilation paths as available.
2. Add the temporary compatibility dependency, regenerate the repository lockfile, and prove a clean external consumer can build `embedded-python`.
3. Add consumer-focused CI checks to pull-request and release gates before publication.
4. Route facade checkpoint through the shared compact turn and replace the placeholder regression assertion with model-driven behavior tests.
5. Publish a patch release and verify a fresh crates.io consumer on Windows and with `embedded-python`.

Rollback can revert the checkpoint routing and CI changes independently. The compatibility pin must not be removed in a rollback unless the published dependency graph is otherwise constrained to a verified compatible set; target gates should remain because removing them restores the Windows linker defect.

## Open Questions

- Should native macOS linking become a required pull-request check now that launchd tests no longer compile on Linux, or is release-time macOS verification sufficient?
- Should the downstream smoke fixture consume an unpacked `cargo package` artifact for maximum fidelity, or is an external path dependency adequate if `cargo package --list` is checked separately?
