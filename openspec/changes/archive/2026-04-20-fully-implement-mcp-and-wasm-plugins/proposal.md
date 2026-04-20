## Why

MCP support is close but still has spec-breaking gaps around approval behavior, runtime-default enablement semantics, and execution-path parity. The Extism/WASM plugin system is much further from complete: the current codebase has scaffolding for registry, manifests, auth, and status, but not a fully integrated install/load/execute/runtime-surface implementation.

## What Changes

- Finish MCP so its session-effective runtime path fully matches the archived specs, especially for approval-strategy handling, runtime-default enablement, accurate unavailable-tool diagnostics, and end-to-end verification.
- Fully implement Extism/WASM plugin loading and execution, including local/remote installation, checksum verification, manifest extraction, runtime inventory, status APIs, auth-mediated availability, and session-effective tool exposure.
- Integrate plugin-backed tools into the same canonical runtime tool surface used by prompt construction, approval, execution, and embedded Python child-tool access.
- Add the missing verification coverage for MCP and plugins so the strongest claims in the specs are backed by integration and end-to-end tests rather than helper-level unit tests alone.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `session-scoped-mcp-support`: align MCP session defaults, approval behavior, and execution diagnostics with the declared runtime contract.
- `python-tool-namespace`: ensure embedded Python child-tool execution follows the same approval and session-effective runtime path for MCP and plugin-backed tools.
- `wasm-integration-plugins`: turn the current plugin scaffolding into a full runtime capability with install/load/execute integration, inventory/status APIs, auth gating, and session-effective tool exposure.

## Impact

- Affected code: `src/runtime.rs`, `src/prompt_runner.rs`, `src/mcp/*`, `src/plugin/*`, `src/facade.rs`, handoff/session state code, and plugin/MCP-related tests.
- Affected APIs: MCP session control and effective-tool behavior, plugin inventory/status APIs, plugin session enablement APIs, and embedded Python runtime tool visibility.
- Affected systems: approval flow, runtime tool catalog composition, Extism/WASM loading/execution, auth mediation, and test infrastructure.
