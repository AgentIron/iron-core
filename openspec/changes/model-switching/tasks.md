## 1. Data Model and Types

- [ ] 1.1 Add `ModelSwitchPlan` struct with source/target model, context adaptation plan, and capability diff
- [ ] 1.2 Add `CapabilityDiff` struct with hidden tools, unsupported modalities, and window shrink info
- [ ] 1.3 Add `ModelSwitched` variant to `TimelineEntry` enum
- [ ] 1.4 Extend `HandoffBundle` with `model_switch_history` and `current_model` fields (backward-compatible)
- [ ] 1.5 Add `ModelSwitchRequest` enum for managed vs unmanaged switching
- [ ] 1.6 Add `ModelSwitchQueue` to `RuntimeSession` for pending switches

## 2. Core Switching Logic

- [ ] 2.1 Implement `AgentSession::switch_model()` method with turn-boundary validation
- [ ] 2.2 Implement `IronRuntime::queue_model_switch()` for active sessions
- [ ] 2.3 Implement `IronRuntime::apply_model_switch()` for idle sessions
- [ ] 2.4 Add turn-completion hook to check and apply queued switches
- [ ] 2.5 Implement `ModelSwitchPlanner::create_plan()` comparing source/target constraints
- [ ] 2.6 Implement context size estimation against target window

## 3. Context Adaptation

- [ ] 3.1 Integrate compaction engine with target window sizing
- [ ] 3.2 Implement pre-switch compaction trigger when target window is smaller
- [ ] 3.3 Implement retained tail sizing based on target window
- [ ] 3.4 Add error path when context cannot fit target window even with minimal tail
- [ ] 3.5 Update `ContextManagementConfig` with model-switch-specific thresholds

## 4. Capability Reconciliation

- [ ] 4.1 Add per-model capability metadata to `iron-providers` (or local registry)
- [ ] 4.2 Implement capability comparison between source and target models
- [ ] 4.3 Update `SessionToolCatalog` to filter tools based on target capabilities
- [ ] 4.4 Implement modality support checking (images, PDFs, etc.)
- [ ] 4.5 Add `CapabilityDiff` generation and client reporting

## 5. Timeline and Metadata

- [ ] 5.1 Record `ModelSwitched` timeline entry on successful switch
- [ ] 5.2 Update `DurableSession` to track current model and switch history
- [ ] 5.3 Update request builder to record per-turn model in metadata
- [ ] 5.4 Update `to_transcript()` to include model switch markers
- [ ] 5.5 Add model switch history to session telemetry

## 6. Client Protocol (ACP)

- [ ] 6.1 Add `ModelSwitchEvent` to client event types
- [ ] 6.2 Add `ModelSwitchPending` notification for queued switches
- [ ] 6.3 Add `CapabilityDiff` payload to switch completion events
- [ ] 6.4 Update `PromptEvent` enum with model switch variants
- [ ] 6.5 Document new ACP messages in protocol docs

## 7. Handoff Bundle Integration

- [ ] 7.1 Update `HandoffExporter` to include model switch metadata
- [ ] 7.2 Update `HandoffImporter` to restore model switch history
- [ ] 7.3 Update `HandoffBundle` serialization tests
- [ ] 7.4 Ensure backward compatibility with v1 bundles

## 8. Tests

- [ ] 8.1 Add unit tests for `ModelSwitchPlanner`
- [ ] 8.2 Add unit tests for capability reconciliation
- [ ] 8.3 Add integration tests for idle session switching
- [ ] 8.4 Add integration tests for active session queuing
- [ ] 8.5 Add tests for context adaptation (larger/smaller window)
- [ ] 8.6 Add tests for timeline recording
- [ ] 8.7 Add tests for handoff bundle round-trip with switches
- [ ] 8.8 Add tests for capability diff generation

## 9. Documentation

- [ ] 9.1 Update API documentation for `AgentSession::switch_model()`
- [ ] 9.2 Document model switching behavior in user guide
- [ ] 9.3 Document capability reconciliation semantics
- [ ] 9.4 Update architecture docs with model switch flow diagram
- [ ] 9.5 Add examples for managed and unmanaged switching

## 10. Frontend Integration (Tauri/SolidJS)

- [ ] 10.1 Add model switch UI in session settings
- [ ] 10.2 Display pending switch status during active turns
- [ ] 10.3 Render `ModelSwitched` timeline entries
- [ ] 10.4 Display capability difference warnings
- [ ] 10.5 Handle switch error states in UI
