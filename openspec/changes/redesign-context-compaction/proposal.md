## Why

Current context compaction is runtime-driven, hidden from the model, and depends on a separate summarization call that lacks the active conversation and tool-use context. This redesign makes compaction an explicit model-visible capability so the active model can preserve the facts, decisions, constraints, file paths, errors, and user intent that matter before old context is removed.

## What Changes

- **BREAKING**: Remove the existing structured `CompactedContext` model and old persisted compacted-session shape without a migration path while the product is still in testing mode.
- **BREAKING**: Remove hidden runtime maintenance/hard-fit compaction that runs a separate summarizer request.
- Add a runtime-owned `compress` tool that lets the active model compact resolved history by selecting validated message/block ranges and providing durable freeform summaries.
- Store compaction output as chronological freeform compressed blocks instead of synthetic replacement timeline entries or structured semantic fields.
- Render stable visible IDs for timeline entries and compressed blocks when compression is available so the model can address ranges precisely.
- Protect active/current context structurally: latest user request, current assistant turn, pending/running tools, and tool call/result pairs cannot be compacted incorrectly.
- Add qualitative context-pressure buckets and model-visible nudges that recommend or require compression without exposing exact telemetry in the prompt.
- Change `/compact` to immediately run a compression-focused model turn with a strong compaction nudge.
- Report a simple user-visible failure when critical pressure remains above threshold after compression attempts, recommending a new session.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `context-compaction`: Replace hidden structured compaction with model-driven freeform compressed blocks, range validation, context-pressure nudges, and `/compact` compression turns.

## Impact

- Affected Rust modules include session durability, timeline representation, request building, prompt/system rendering, prompt runner turn orchestration, tool dispatch, and slash command handling.
- Provider requests will include chronological compressed blocks plus retained active transcript instead of `CompactedContext` plus retained tail.
- Existing compacted testing sessions may break because `CompactedContext` is removed without migration.
- Existing compaction tests and specs must be updated to assert freeform compressed block rendering, range safety, and pressure/nudge behavior.
