## 1. MCP correctness completion

- [x] 1.1 Update new-session MCP initialization to use the single runtime-level default policy rather than per-server default override semantics
- [x] 1.2 Apply `ApprovalStrategy::is_approval_required(...)` in the canonical session-effective MCP execution path for model-issued and child-tool calls
- [x] 1.3 Improve session-effective MCP unavailable-tool diagnostics so disabled, unhealthy, and unknown-tool cases are reported precisely
- [x] 1.4 Add targeted MCP tests covering runtime-default enablement semantics, approval-strategy behavior, and precise unavailable-tool errors

## 2. Canonical runtime tool catalog expansion

- [x] 2.1 Extend `SessionToolCatalog` and related runtime inspection APIs to represent plugin-backed tools alongside local and MCP tools
- [x] 2.2 Route prompt construction, approval checks, runtime execution, and embedded Python visibility through the expanded canonical session-effective tool catalog
- [x] 2.3 Add regression tests proving provider-visible tools, public effective-tool inspection, and execution-visible tools stay aligned for local, MCP, and plugin-backed tools

## 3. Plugin installation and runtime inventory

- [x] 3.1 Implement plugin lifecycle loading for local file sources, including manifest load/extraction and runtime-owned installed plugin state
- [x] 3.2 Implement HTTPS plugin fetch with required checksum verification and rejection for missing or mismatched checksums
- [x] 3.3 Separate plugin runtime health/install state from per-session enablement defaults in the plugin registry and status model
- [x] 3.4 Expose client-visible runtime plugin inventory, per-plugin metadata, and runtime status through the public runtime/facade surfaces

## 4. Extism/WASM execution and auth-mediated availability

- [x] 4.1 Implement Extism host execution for plugin-backed tool calls with schema validation, structured error mapping, and result propagation
- [x] 4.2 Compute plugin effective tool availability centrally from runtime health, session enablement, auth state, per-tool scopes, and approval requirements
- [x] 4.3 Expose per-tool availability and auth-mediated status details through plugin inspection APIs
- [x] 4.4 Implement runtime-owned auth-state updates and availability recomputation for plugin auth transitions

## 5. Embedded Python and session control integration

- [x] 5.1 Ensure embedded Python child-tool execution uses the same approval and session-effective path for plugin-backed and MCP-backed tools
- [x] 5.2 Add session-scoped plugin enable/disable controls to the public runtime/facade surface and verify isolation across sessions
- [x] 5.3 Add handoff tests confirming plugin inventory, auth bindings, and session plugin enablement remain excluded from portability

## 6. Verification

- [x] 6.1 Add plugin integration and end-to-end tests covering install/load, effective visibility, execution, auth-gated availability, and client inspection
- [x] 6.2 Re-run MCP, plugin, ACP runtime, and embedded-Python regression suites after the full implementation lands
- [x] 6.3 Update any outdated placeholder tests so the strongest MCP and plugin claims are backed by high-signal integration coverage