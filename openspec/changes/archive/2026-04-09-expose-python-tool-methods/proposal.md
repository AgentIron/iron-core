## Why

The embedded Python runtime currently exposes only a generic `iron_call(name, args)` escape hatch, which makes tool orchestration harder for the model than normal top-level tool calling. The runtime already has a canonical visible tool surface in `ToolRegistry`, so Python should expose that same surface directly instead of forcing name-string dispatch.

## What Changes

- Expose a Python tool namespace that provides one callable entry per visible runtime tool in `ToolRegistry`.
- Define `ToolRegistry` as the canonical source for the Monty-visible tool catalog, covering built-in tools, custom registered tools, and capability-backed tools that are presented as tool entries.
- Preserve a raw-name fallback for tools whose names are not valid Python identifiers.
- Exclude purely metadata-only capability descriptors from the Python surface unless they are represented as executable tools.
- Document that capability negotiation must materialize as tool registration or replacement if the capability is intended to be callable from Python.

## Capabilities

### New Capabilities
- `python-tool-namespace`: Defines the embedded Python tool surface, discovery model, and execution contract for runtime tools.

### Modified Capabilities

## Impact

- Affected code: `src/embedded_python/*`, `src/prompt_runner.rs`, `src/tool.rs`, runtime/tool registration paths, and embedded Python tests.
- Affected behavior: how `python_exec` scripts discover and invoke tools, and how capability-backed tools become visible inside Monty.
- No external dependency changes are expected.
