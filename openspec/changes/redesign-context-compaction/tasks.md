## 1. Durable Data Model

- [x] 1.1 Remove `CompactedContext` storage and request-rendering dependencies from durable session state.
- [x] 1.2 Add durable freeform compressed block storage with block ID, topic, source range, summary, creation time, and optional telemetry fields.
- [x] 1.3 Add stable internal IDs for durable timeline entries and maintain visible ID mapping for provider-rendered context.
- [x] 1.4 Update session serialization/deserialization for the new compressed block shape without adding a migration path for old compacted sessions.

## 2. Provider Request Rendering

- [x] 2.1 Render chronological compressed blocks before retained active transcript in provider requests.
- [x] 2.2 Render compact visible IDs for timeline entries and compressed blocks when compression is available or useful.
- [x] 2.3 Remove synthetic compacted-context message rendering and all `CompactedContext` request composition paths.
- [x] 2.4 Ensure activated skills remain in the instruction layer and are not included in compressed block summaries.

## 3. Pressure And Prompt Nudges

- [x] 3.1 Add qualitative context-pressure buckets for none, soft, medium, strong, and critical states.
- [x] 3.2 Compute exact context telemetry internally while rendering only qualitative pressure guidance to the model.
- [x] 3.3 Add system prompt rendering for compression availability, pressure nudges, and compress-summary durability instructions.
- [x] 3.4 Update prompt cache fingerprinting so exact telemetry changes do not invalidate cache without bucket or availability changes.
- [x] 3.5 Recompute pressure after compression and clear nudges only when provider-visible usage falls below the active threshold.

## 4. Compress Tool Execution

- [x] 4.1 Define the runtime-owned `compress` tool schema with topic and one or more source ranges containing summaries.
- [x] 4.2 Expose `compress` in the effective tool catalog only when compression is available or useful.
- [x] 4.3 Intercept `compress` execution before normal MCP, plugin, Python, approval, or child-tool dispatch.
- [x] 4.4 Validate all requested ranges before mutation and reject the whole call on any invalid range.
- [x] 4.5 Enforce structural protections for tool call/result pairs, latest user request, current assistant turn, pending/running tools, pending-approval tools, unknown IDs, reversed ranges, overlapping ranges, and already-removed IDs.
- [x] 4.6 Apply valid compression by removing selected active transcript entries and selected older compressed blocks, then appending new chronological compressed blocks.
- [x] 4.7 Return tool results that report blocks created, validation failures, and current qualitative pressure state.

## 5. Runtime Flow And Slash Command

- [x] 5.1 Remove hidden runtime maintenance/hard-fit compaction execution and separate summarizer calls.
- [x] 5.2 Update critical pressure handling to request model-driven compression rather than running hidden fallback compaction.
- [x] 5.3 Implement `/compact` as an immediate compression-focused model turn with a strong compression nudge.
- [x] 5.4 Surface a simple user-visible error when critical pressure cannot be reduced below threshold after the allowed compression attempt.

## 6. Tests And Verification

- [x] 6.1 Add unit tests for compressed block storage, rendering order, and visible block IDs.
- [x] 6.2 Add unit tests for range validation, including tool pair splitting, active context protection, overlapping ranges, unknown IDs, and already-removed IDs.
- [x] 6.3 Add integration tests proving provider requests include compressed blocks and exclude compressed historical active transcript/tool traffic.
- [x] 6.4 Add tests for pressure bucket rendering, prompt cache bucket behavior, and nudge clearing only after recomputed usage is below threshold.
- [x] 6.5 Add tests for `/compact` immediate compression turn behavior and critical-pressure failure messaging.
- [x] 6.6 Run `cargo check` and the relevant Rust tests for this crate.
