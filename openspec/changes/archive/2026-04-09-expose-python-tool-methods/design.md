## Context

The embedded Python runtime currently exposes a single generic entry point, `await iron_call(name, args)`, for tool invocation. That works mechanically, but it is a poor ergonomic fit for model-driven orchestration because the model has to reason about stringly typed dispatch instead of the actual callable tool surface.

At the runtime level, `ToolRegistry` already represents the canonical set of tools the model can call. It includes built-in tools, custom registered tools, and any future capability-backed implementations that are presented as actual tools. In contrast, `CapabilityRegistry` currently acts as metadata and backend-selection state rather than an executable callable catalog. That means the embedded Python surface should follow `ToolRegistry`, not `CapabilityRegistry` directly.

The design target is a Monty-visible tool namespace that mirrors the current runtime tool catalog while preserving the existing execution path for schema validation, permission handling, durable tool recording, and result normalization.

## Goals / Non-Goals

**Goals:**
- Make every tool currently visible in `ToolRegistry` callable from embedded Python through an ergonomic namespace.
- Keep `ToolRegistry` as the only canonical source of callable Python tools.
- Support built-in tools, custom tools, and capability-backed tools as long as they materialize as tool entries.
- Preserve a raw-name fallback for tools whose names cannot be expressed safely as Python identifiers.
- Reuse the existing child-tool execution path so Python tool calls inherit current validation, permission, and durable-recording behavior.
- Improve discoverability for the model through namespace introspection and method-level descriptions.

**Non-Goals:**
- Supporting arbitrary ACP-advertised remote tools outside `ToolRegistry`.
- Making `CapabilityRegistry` itself directly callable from Python.
- Changing top-level provider tool calling semantics outside `python_exec`.
- Expanding Monty into a full dynamic Python object system beyond what is needed for tool exposure.

## Decisions

### 1. `ToolRegistry` SHALL be the canonical Monty tool catalog

The Python-visible tool namespace will be derived from the currently visible `ToolRegistry` entries at script start.

Rationale:
- `ToolRegistry` already reflects what the runtime can execute.
- It keeps built-ins, custom tools, and capability-backed tools under one callable abstraction.
- It avoids exposing metadata-only capabilities that are not actually executable.

Alternatives considered:
- Derive the Python surface from `CapabilityRegistry`: rejected because capabilities are not the execution surface today.
- Maintain a second Python-specific registry: rejected because it creates drift from the actual runtime tool surface.

### 2. Embedded Python SHALL expose a namespace object, not only a generic function

Scripts should receive a namespace object such as `tools` with one callable member per visible tool, plus a raw-name fallback like `tools.call(name, payload)`.

Rationale:
- This is closer to how models reason about tool use.
- A namespace avoids polluting globals and reduces naming collisions.
- The raw fallback preserves completeness when tool names are not valid identifiers.

Alternatives considered:
- Keep only `iron_call(name, args)`: rejected because it preserves the current ergonomics problem.
- Generate flat global functions: rejected because global names are more collision-prone and harder to evolve.

### 3. Tool names SHALL support both sanitized aliases and raw-name dispatch

For each tool in the catalog, the runtime should expose:

- an identifier-safe alias when possible
- a lossless raw-name fallback through `tools.call(...)`

Rationale:
- Tool names may contain characters or forms that are awkward for Python identifiers.
- The system must remain complete even when alias generation is imperfect.

Alternatives considered:
- Expose only sanitized names: rejected because aliasing can be lossy or ambiguous.
- Expose only raw-name dispatch on the namespace: rejected because it loses the main ergonomic benefit.

### 4. Namespace contents SHALL be snapshotted per script run

The embedded Python runtime should expose a stable view of the tool catalog for the lifetime of a single script execution.

Rationale:
- Mid-script tool catalog mutation would be confusing and difficult for models to reason about.
- A per-run snapshot is easier to test and document.

Alternatives considered:
- Dynamic live lookup on every attribute access: acceptable in principle, but rejected because it makes script behavior depend on mutable runtime state during execution.

### 5. Python tool methods SHALL reuse the existing child-call executor path

Generated Python callables should delegate into the current embedded Python child-tool execution pipeline rather than bypassing it.

Rationale:
- This preserves schema validation.
- This preserves permission behavior.
- This preserves durable tool records and script activity reporting.
- This minimizes new execution logic.

Alternatives considered:
- Call tool implementations directly from the Monty bridge: rejected because it would duplicate orchestration logic and create behavioral drift.

### 6. Capability negotiation SHALL only affect Python visibility through tool registration

If a client-negotiated capability backend is meant to be callable from Python, it must appear in `ToolRegistry` as the corresponding tool implementation or replacement. Metadata-only capability entries do not become Python methods.

Rationale:
- This keeps one consistent callable model.
- It allows backend substitution without changing model-facing tool names.

Alternatives considered:
- Expose capability-specific Python APIs separately from tools: rejected because it creates two invocation models for the same conceptual operation.

### 7. Recursive `python_exec` exposure should be treated deliberately

The design should explicitly decide whether `python_exec` appears inside the generated namespace. The safer default is to exclude it from alias generation while still allowing raw fallback only if intentionally supported.

Rationale:
- Recursive script spawning is easy to misuse and may create confusing control flow.

## Risks / Trade-offs

- [Alias generation creates collisions between tool names] -> Define deterministic sanitization and require raw-name fallback for lossless access.
- [The Python namespace drifts from the actual runtime tool surface] -> Build it from a per-run `ToolRegistry` snapshot rather than a separate registry.
- [Models may misuse recursive `python_exec`] -> Exclude or explicitly gate `python_exec` in the generated namespace.
- [Capability overrides remain metadata-only and do not show up in Python as expected] -> Document that callable capability negotiation must materialize as tool registration or substitution.
- [Monty object-model constraints make rich namespace behavior awkward] -> Keep the surface minimal: callable aliases, raw fallback, and basic introspection.

## Migration Plan

1. Add a tool-catalog snapshot model for embedded Python derived from `ToolRegistry`.
2. Introduce a Monty-visible namespace object that exposes callable tool aliases plus raw-name fallback.
3. Route generated methods through the existing child-tool executor path.
4. Add tests for built-in tools, custom tools, alias/fallback behavior, and capability-backed tool visibility.
5. Update `python_exec` prompt/runtime documentation to describe the new namespace and its limits.

Rollback strategy:
- Revert to the existing `iron_call(name, args)`-only interface if integration issues emerge.

## Open Questions

- Should `iron_call(name, args)` remain available as a backward-compatible escape hatch once the namespace exists?
- Should `python_exec` be fully hidden from the namespace, or merely omitted from aliases while remaining available through raw fallback?
- How much introspection is worth supporting initially: just `tools.call`, or also `tools.available()` and `tools.describe(name)`?
