## 1. Prompt And Tool Guidance

- [x] 1.1 Update `src/prompt/baseline.rs` so `python_exec` is described as a sandboxed orchestration tool and single-tool work is not unnecessarily routed through it.
- [x] 1.2 Update `src/prompt/runtime_context.rs` to state that direct OS/filesystem/environment access is unavailable inside `python_exec` and that host interactions must use `tools.<alias>(payload)` or `tools.call(name, payload)`.
- [x] 1.3 Update `src/embedded_python/python_exec_tool.rs` so the tool description reinforces the sandbox boundary and points models away from direct APIs such as `pathlib`, `open`, and `os`.

## 2. Sandbox Failure Messaging

- [x] 2.1 Update the embedded Python `OsCall` failure path in `src/embedded_python/engine.rs` to return sandbox-violation guidance that directs the caller to visible runtime tools.
- [x] 2.2 Ensure the user-visible `python_exec` result surface preserves the actionable sandbox-violation classification and recovery message.

## 3. Verification

- [x] 3.1 Add or update prompt/runtime-context tests to verify the generated guidance includes the sandbox boundary and the `tools`-namespace recovery path.
- [x] 3.2 Add or update embedded Python tests to verify unsupported direct OS access fails with actionable sandbox-violation messaging.
- [x] 3.3 Run the relevant embedded Python and prompt-composition test coverage for the updated behavior.
