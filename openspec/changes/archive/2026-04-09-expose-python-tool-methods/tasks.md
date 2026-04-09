## 1. Tool Catalog Modeling

- [x] 1.1 Define an embedded-Python tool catalog snapshot derived from `ToolRegistry` at script start.
- [x] 1.2 Define deterministic aliasing rules for Python-safe tool names and a raw-name fallback for complete access.

## 2. Monty Namespace Exposure

- [x] 2.1 Extend the Monty bridge to expose a `tools` namespace with callable entries for visible runtime tools.
- [x] 2.2 Decide and implement the treatment of `python_exec` within the generated namespace.
- [x] 2.3 Add minimal discovery support such as raw-name dispatch and any selected introspection helpers.

## 3. Execution Integration

- [x] 3.1 Route namespace-exposed Python tool calls through the existing child-tool execution path.
- [x] 3.2 Ensure capability-backed tools become Python-visible only when they are materialized as `ToolRegistry` entries.

## 4. Verification

- [x] 4.1 Add tests proving built-in tools appear in the Python namespace.
- [x] 4.2 Add tests proving custom registered tools appear in the Python namespace.
- [x] 4.3 Add tests covering alias generation and raw-name fallback behavior.
- [x] 4.4 Add tests proving namespace calls inherit existing validation, permission, and durable child-call behavior.
- [x] 4.5 Add tests proving the tool namespace is snapshotted for the lifetime of a script run.

## 5. Documentation

- [x] 5.1 Update `python_exec` descriptions and runtime guidance to describe the `tools` namespace.
- [x] 5.2 Clarify that callable client-negotiated capability backends must materialize as tool registration or substitution to appear in Python.
