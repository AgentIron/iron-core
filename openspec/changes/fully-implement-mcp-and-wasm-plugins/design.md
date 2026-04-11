## Context

The codebase now has two adjacent runtime-extension systems in different states of maturity.

MCP support is largely functional: session-scoped visibility, runtime-owned connection management, stdio execution, HTTP+SSE correlation, reconnect behavior, and embedded Python child-tool access all exist. The remaining MCP issues are contract mismatches in the canonical session-effective execution path, especially around approval-strategy handling and how new sessions derive enablement defaults.

The Extism/WASM plugin system is mostly scaffolded. The repository already contains plugin configuration, manifest validation, registry/state models, auth and status enums, and some helper logic, but the runtime does not yet treat plugin-backed tools as first-class members of the canonical tool surface. Installation, remote fetch, manifest extraction, tool execution, client-visible inspection, and session-effective execution integration are incomplete or stubbed.

The main architectural challenge is that both MCP and plugins must converge on the same runtime truth: one session-effective tool catalog, one approval strategy path, one durable tool-record lifecycle, and one public inspection model.

## Goals / Non-Goals

**Goals:**
- Make the canonical MCP/session-effective execution path honor the configured approval strategy and the runtime-level default enablement semantics described in the spec.
- Extend the canonical session-effective tool catalog to include plugin-backed tools alongside local and MCP tools.
- Fully implement Extism/WASM plugin install/load/execute behavior for local and remote plugin sources with checksum verification.
- Expose client-visible plugin inventory, status, per-tool availability, session enablement, and auth state in a runtime-owned way.
- Ensure embedded Python sees and executes MCP and plugin-backed tools through the same session-effective runtime path used by normal model-issued tool calls.
- Add integration and end-to-end verification strong enough to support the specs for both MCP and plugins.

**Non-Goals:**
- Supporting non-Extism plugin runtimes in this change.
- Designing a plugin-defined custom auth lifecycle vocabulary beyond the runtime-standardized v1 auth model.
- Expanding MCP beyond its current tool-focused scope into broader protocol capabilities such as resources or prompts.

## Decisions

### Extend one canonical `SessionToolCatalog` instead of adding parallel plugin paths
The runtime should keep a single session-effective tool catalog that aggregates local tools, MCP-backed tools, and plugin-backed tools. Prompt construction, approval checks, execution, and embedded Python should all consume this same catalog.

This is preferable to keeping separate plugin helper surfaces because the MCP audit already showed how divergence creates spec and regression problems.

### Approval strategy must be applied at the session-effective layer, not only at legacy execution paths
The session-effective prompt/execution path should call `ApprovalStrategy::is_approval_required(...)` rather than directly using `requires_approval()` from a tool definition. This must apply equally to model-issued calls, MCP-backed calls, plugin-backed calls, and embedded Python child-tool calls.

This is preferable to tool-specific branching because it preserves the runtime-wide approval contract and fixes the observed ACP regressions.

### MCP enablement should use a single runtime default for new sessions
New sessions should initialize MCP enablement from the runtime-level policy described in the spec. Per-server MCP configuration can still exist as metadata, but it should no longer override the single runtime default when new session state is initialized.

This is preferable to the current mixed model because the spec promises a single runtime default and the current behavior is both surprising and hard to verify.

### Plugin runtime health must be independent from per-session enablement defaults
Plugin runtime registry state should represent whether a plugin is installed, loaded, healthy, errored, or auth-gated. Per-session enablement should remain separate durable session state. A plugin being disabled by default for new sessions must not make the runtime inventory report the plugin as runtime-disabled or unhealthy.

This mirrors the MCP separation between runtime inventory and session intent and avoids exposing misleading state to clients.

### Plugin installation must produce a concrete runtime-owned plugin artifact
The plugin lifecycle should normalize local and remote sources into a validated local artifact, verify required checksums for remote sources, load or extract the plugin manifest, and register a runtime-owned plugin state that includes exported tool metadata and runtime status.

This is preferable to lazy or implicit loading because it gives the runtime a stable inventory and clearer failure points for clients.

### Extism execution should be mediated through a narrow host layer
Plugin-backed tool execution should be delegated through a dedicated Extism host adapter that validates input, invokes the plugin entrypoint, maps plugin errors into runtime errors, and returns structured results back through the standard runtime tool lifecycle.

This is preferable to scattering host logic across registry or prompt code because it isolates Extism-specific concerns and makes testing easier.

### Plugin auth/tool availability should be computed centrally
The runtime should compute plugin health, auth availability, and per-tool availability in one place, then feed that into the session-effective tool catalog and client-visible inspection APIs. Per-tool scope requirements and approval requirements must be part of that computation.

This is preferable to exposing raw manifest data directly because the spec requires the runtime to be authoritative for effective availability.

### Verification should be integration-heavy, not only helper-level
For MCP and plugins, strong verification should happen through runtime/session/inference flows, not only unit tests of registries or helper structs. At minimum, the new tests should cover approval strategy behavior, plugin session visibility, plugin execution, remote checksum failure/success behavior, auth-gated availability recomputation, embedded Python child-tool parity, and handoff exclusion.

## Risks / Trade-offs

- [Adding plugins to the canonical tool catalog increases cross-cutting impact] → Keep the catalog interface explicit and add focused integration tests for each tool source type.
- [Extism host integration introduces runtime and dependency complexity] → Isolate the host adapter behind a narrow module boundary and test it with fake/minimal plugins.
- [Remote plugin loading can introduce security and supply-chain risk] → Require checksums for remote sources, validate manifests strictly, and keep runtime-owned metadata separate from untrusted plugin claims.
- [Centralized auth/availability computation can become overly coupled] → Use explicit data structures for plugin health, auth availability, and per-tool availability rather than ad hoc booleans spread across the codebase.
- [Changing MCP default semantics may affect existing assumptions] → Update tests and docs together, and make the runtime-level policy the only source of truth for new-session MCP defaults.

## Migration Plan

- Correct MCP approval/default behavior in the canonical session-effective path first, since those are small but correctness-critical regressions.
- Extend `SessionToolCatalog` and related runtime APIs to represent plugin-backed tools.
- Implement plugin installation/loading and Extism execution behind runtime-owned lifecycle and registry components.
- Expose client-visible plugin inventory/status/session control APIs and integrate auth-mediated availability.
- Update embedded Python to rely on the expanded session-effective tool catalog.
- Add or strengthen integration/e2e tests for MCP and plugins, then rerun full regression suites.

## Open Questions (resolved)

- ~~Should the existing per-server MCP `enabled_by_default` field be removed entirely, or preserved only as non-authoritative metadata for compatibility?~~ **Resolved:** The per-server field is preserved as metadata but is no longer authoritative for new-session defaults. The single runtime-level `enabled_by_default` is the only source of truth for new sessions.

- ~~Which minimal Extism contract should the runtime require for v1 plugin execution (manifest embedding, exported function names, and error envelope shape)?~~ **Resolved:** The v1 contract requires: (1) a WASM custom section named `iron_manifest` containing UTF-8 JSON, (2) tool entrypoints exported as `tool_{tool_name}`, (3) request envelope: JSON via Extism input buffer, (4) response envelope: `{"ok": <value>}` or `{"error": "<message>"}`, (5) 30-second timeout via Extism manifest. This is documented in `src/plugin/wasm_host.rs`.

- ~~Should plugin installation be eager at registration time only, or support a lazy-but-runtime-owned load state that still presents accurate client-visible inventory?~~ **Resolved:** Installation is eager — `install_with_loader()` processes the artifact, extracts the manifest, validates it, and hands off to the WASM host in a single pipeline. The registry presents accurate inventory at all times because incomplete installs roll back to `Error` health.
