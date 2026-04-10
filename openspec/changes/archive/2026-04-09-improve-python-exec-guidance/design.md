## Context

`iron-core` already uses the correct execution boundary for embedded Python tool orchestration. Calls made through `tools.<alias>(...)` and `tools.call(name, ...)` leave Monty and run through the normal child-tool pipeline, which preserves schema validation, approval handling, durable recording, and tool-specific permission policy.

The user experience problem is at the model-contract layer. The baseline prompt tells the model to prefer `python_exec` for orchestration and parallel work, but the current prompt fragments and tool description do not state strongly enough that `python_exec` is still a sandboxed scripting environment. As a result, models can choose inline Python filesystem or OS APIs such as `pathlib`, `open`, or `os`, hit Monty's `OsCall` boundary, and surface a generic sandbox failure instead of using the already-correct runtime tools.

This change should improve guidance without changing the underlying trust model. `iron-core` should continue to keep Monty sandboxed and continue to route host access through registered tools.

## Goals / Non-Goals

**Goals:**
- Make the model-facing contract for `python_exec` explicit: it is for computation and orchestration, not direct host I/O.
- Teach models that filesystem, shell, network, and other host interactions from embedded Python MUST go through `tools.*` or `tools.call(...)`.
- Reduce avoidable sandbox failures by making prompt guidance, tool descriptions, and runtime failures reinforce the same rule.
- Preserve the current child-tool execution path and approval model.
- Add tests that lock in the expected guidance and recovery messaging.

**Non-Goals:**
- Changing how top-level tool calls work outside `python_exec`.
- Granting Monty direct filesystem or OS access.
- Building a compatibility layer that translates arbitrary Monty `OsCall` operations into runtime tool calls.
- Expanding the embedded Python surface beyond the current `tools` namespace and low-level `iron_call` escape hatch.

## Decisions

### 1. The prompt contract SHALL distinguish orchestration from host access

The baseline prompt and runtime context will explicitly describe `python_exec` as a sandboxed orchestration environment. They will state that direct host access from Python is unavailable and that any filesystem, shell, network, or similar side effect must go through runtime tools.

Rationale:
- The model sees prompt guidance at decision time; repo documentation alone cannot prevent misuse.
- The current recommendation to prefer `python_exec` needs a companion rule that explains where host access belongs.
- Repeating the same boundary in the baseline prompt and runtime context makes the intended pattern more robust across providers.

Alternatives considered:
- Rely on tool descriptions only: rejected because prompt guidance drives the initial choice to enter `python_exec`.
- Avoid recommending `python_exec`: rejected because it is still the right mechanism for deterministic orchestration and parallel tool coordination.

### 2. The `python_exec` tool description SHALL describe the unsupported path explicitly

The `python_exec` tool description will say that the runtime is sandboxed, that host interactions must use the `tools` namespace, and that direct APIs such as `pathlib`, `open`, and `os` are not the supported path for workspace access.

Rationale:
- Tool descriptions are part of the model-visible contract and should reinforce the same boundary as the prompt.
- Naming likely failure patterns directly is more effective than describing only the happy path.

Alternatives considered:
- Keep the description focused only on orchestration ergonomics: rejected because that wording contributes to the current ambiguity.

### 3. Unsupported direct OS access SHALL fail with recovery-oriented guidance

When embedded Python attempts Monty `OsCall` host access that `iron-core` does not support, the returned failure should identify the violation as a sandbox-boundary issue and tell the model to use runtime tools instead.

Rationale:
- Generic sandbox errors are accurate but not corrective.
- The runtime already knows the right recovery path: use visible tools through `tools.*`.
- Better error messages improve both autonomous retries and user-facing clarity.

Alternatives considered:
- Keep the current generic runtime error: rejected because it does not guide repair.
- Translate Monty `OsCall` operations into runtime tool calls: rejected for this change because it would add a second host-access path and substantially broaden scope.

### 4. Verification SHALL cover both guidance surfaces and failure surfaces

Tests will verify that generated prompt/runtime guidance includes the sandbox boundary and that unsupported direct OS access returns actionable recovery messaging.

Rationale:
- This change is primarily model-contract behavior, so tests need to lock in wording-sensitive surfaces.
- Existing embedded Python tests already cover orchestration mechanics; this change should add targeted coverage for steerability.

Alternatives considered:
- Test only the final runtime error string: rejected because prompt regressions would remain undetected.

## Risks / Trade-offs

- [Prompt wording becomes too verbose or repetitive] -> Keep the guidance short and rule-based, and place the detailed boundary explanation in the runtime-context section where `python_exec` is already described.
- [Model still attempts direct Python I/O despite better guidance] -> Return a recovery-oriented sandbox-violation error that points back to `tools.*`.
- [Examples of unsupported APIs become stale or incomplete] -> Name a few high-frequency cases (`pathlib`, `open`, `os`) and describe the general rule in terms of host access rather than an exhaustive denylist.
- [Future support for host-mediated `OsCall` handling conflicts with this wording] -> Phrase the contract around the supported path today: host access from embedded Python goes through visible runtime tools.

## Migration Plan

1. Update the baseline prompt and embedded Python runtime-context text to define `python_exec` as sandboxed orchestration and route host access to `tools.*`.
2. Update the `python_exec` tool description to reinforce the same contract and mention common unsupported direct APIs.
3. Change the embedded Python `OsCall` failure path to return actionable sandbox-violation guidance.
4. Add or update tests for prompt composition, runtime guidance, and embedded Python sandbox failures.

Rollback strategy:
- Revert the prompt and error-message changes without affecting the underlying tool-execution architecture.

## Open Questions

- None. This change intentionally keeps scope to prompt contract and failure messaging rather than changing execution semantics.
