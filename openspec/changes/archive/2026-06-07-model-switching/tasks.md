# Model Switching Tasks

## 1. Data Model and Types

- [x] 1.1 Add `ModelSwitchPlan` struct with source/target model, context adaptation plan, and capability diff
- [x] 1.2 Add `CapabilityDiff` struct with hidden tools, unsupported modalities, and window shrink info
- [x] 1.3 Add `ModelSwitched` variant to `TimelineEntry` enum
- [x] 1.4 Extend `HandoffBundle` with `model_switch_history` and `current_model` fields (backward-compatible)
- [x] 1.5 Add `ModelSwitchRequest` enum for managed vs unmanaged switching
- [x] 1.6 Add `ModelSwitchQueue` to `RuntimeSession` for pending switches

## 2. Core Switching Logic

- [x] 2.1 Implement `AgentSession::switch_model()` method with turn-boundary validation
- [x] 2.2 Implement `IronRuntime::queue_model_switch()` for active sessions
- [x] 2.3 Implement `IronRuntime::apply_model_switch()` for idle sessions
- [x] 2.4 Add turn-completion hook to check and apply queued switches
- [x] 2.5 Implement `ModelSwitchPlanner::create_plan()` comparing source/target constraints
- [x] 2.6 Implement context size estimation against target window

## 3. Context Adaptation

- [x] 3.1 Integrate compaction engine with target window sizing
- [x] 3.2 Implement pre-switch compaction trigger when target window is smaller
- [x] 3.3 Implement retained tail sizing based on target window
- [x] 3.4 Add error path when context cannot fit target window even with minimal tail
- [x] 3.5 Update `ContextManagementConfig` with model-switch-specific thresholds

## 4. Capability Reconciliation

- [x] 4.1 Add per-model capability metadata to `iron-providers` (or local registry)
- [x] 4.2 Implement capability comparison between source and target models
- [x] 4.3 Update `SessionToolCatalog` to filter tools based on target capabilities
- [x] 4.4 Implement modality support checking (images, PDFs, etc.)
- [x] 4.5 Add `CapabilityDiff` generation and client reporting

## 5. Timeline and Metadata

- [x] 5.1 Record `ModelSwitched` timeline entry on successful switch
- [x] 5.2 Update `DurableSession` to track current model and switch history
- [x] 5.3 Update request builder to record per-turn model in metadata
- [x] 5.4 Update `to_transcript()` to include model switch markers
- [x] 5.5 Add model switch history to session telemetry

## 6. Client Protocol (ACP)

- [x] 6.1 Add `ModelSwitchEvent` to client event types
- [x] 6.2 Add `ModelSwitchPending` notification for queued switches
- [x] 6.3 Add `CapabilityDiff` payload to switch completion events
- [x] 6.4 Update `PromptEvent` enum with model switch variants
- [x] 6.5 Document new ACP messages in protocol docs

## 7. Handoff Bundle Integration

- [x] 7.1 Update `HandoffExporter` to include model switch metadata
- [x] 7.2 Update `HandoffImporter` to restore model switch history
- [x] 7.3 Update `HandoffBundle` serialization tests
- [x] 7.4 Ensure backward compatibility with v1 bundles

## 8. Tests

- [x] 8.1 Add unit tests for `ModelSwitchPlanner`
- [x] 8.2 Add unit tests for capability reconciliation
- [x] 8.3 Add integration tests for idle session switching
- [x] 8.4 Add integration tests for active session queuing
- [x] 8.5 Add tests for context adaptation (larger/smaller window)
- [x] 8.6 Add tests for timeline recording
- [x] 8.7 Add tests for handoff bundle round-trip with switches
- [x] 8.8 Add tests for capability diff generation

## 9. Documentation

- [x] 9.1 Update API documentation for `AgentSession::switch_model()`
- [x] 9.2 Document model switching behavior in user guide
- [x] 9.3 Document capability reconciliation semantics
- [x] 9.4 Update architecture docs with model switch flow diagram
- [x] 9.5 Add examples for managed and unmanaged switching

## 10. Frontend Integration (Tauri/SolidJS)

- [x] 10.1 Add model switch UI in session settings
- [x] 10.2 Display pending switch status during active turns
- [x] 10.3 Render `ModelSwitched` timeline entries
- [x] 10.4 Display capability difference warnings
- [x] 10.5 Handle switch error states in UI

_Note: Frontend integration tasks are implemented in the consuming application. `iron-core` provides the full backend support including `PromptEvent::ModelSwitched`, `PromptEvent::ModelSwitchPending`, capability diffs, and model switch history for the frontend to consume._
