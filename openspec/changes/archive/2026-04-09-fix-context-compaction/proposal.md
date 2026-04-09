## Why

`iron-core`'s context compaction feature does not currently preserve useful continuity in future prompts. The code can generate a `compacted_context`, but that summary is not consistently fed back into subsequent provider requests, and old tool history can remain provider-visible even after compaction.

## What Changes

- Redefine compaction so future prompts are built from `compacted_context + retained tail` rather than retained tail plus historical tool transcript.
- Make compaction destructive for pre-tail session history within this feature's scope, including old tool-call records that have already been summarized.
- Ensure compaction triggers reflect the actual provider-visible context growth, including tool-heavy sessions.
- Add verification that compaction reduces future prompt context while preserving semantic continuity.

## Capabilities

### New Capabilities
- `context-compaction`: Defines how compacted session state is constructed, retained, and re-used in future prompts.

### Modified Capabilities

## Impact

- Affected code: `src/context/*`, `src/durable.rs`, `src/prompt_runner.rs`, `src/request_builder.rs`, `src/facade.rs`, and context-management integration tests.
- Affected behavior: future provider requests, post-turn maintenance compaction, hard-fit compaction, checkpoint compaction, and session history visibility after compaction.
- No external dependency changes are expected.
