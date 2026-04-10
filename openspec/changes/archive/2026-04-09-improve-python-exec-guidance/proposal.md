## Why

`iron-core` correctly routes `python_exec` tool invocations through the normal child-tool path, but the current prompt and tool guidance do not state the sandbox boundary strongly enough. Models are told to prefer `python_exec` for orchestration, yet they are not told clearly enough that host access must still go through runtime tools, which leads to avoidable `OS access not available in sandbox` failures and a poor user experience.

## What Changes

- Tighten the baseline prompt, runtime context, and `python_exec` tool description so they describe `python_exec` as a sandboxed orchestration environment rather than a general host Python environment.
- Explicitly instruct models to use `tools.<alias>(payload)` or `tools.call(name, payload)` for filesystem, shell, network, and other host interactions from embedded Python.
- Clarify that direct host APIs such as `pathlib`, `open`, and `os` are not the supported path for workspace access inside `python_exec`.
- Improve sandbox-violation failures so they direct the model toward the appropriate runtime tools instead of surfacing a generic sandbox error.
- Add coverage that verifies the generated guidance and failure messages steer models toward the tool namespace.

## Capabilities

### New Capabilities

### Modified Capabilities
- `python-tool-namespace`: Strengthen the documented contract so embedded Python guidance clearly distinguishes sandboxed script execution from host-access tool calls and provides actionable recovery when a script attempts direct OS access.

## Impact

- Affected code: `src/prompt/baseline.rs`, `src/prompt/runtime_context.rs`, `src/embedded_python/python_exec_tool.rs`, `src/embedded_python/engine.rs`, and embedded Python prompt/runtime tests.
- Affected behavior: model-facing instructions for `python_exec`, tool descriptions exposed to providers, and the error surface when embedded Python attempts unsupported direct OS access.
- No external dependency changes are expected.
