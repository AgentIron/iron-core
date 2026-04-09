## Context

`iron-core` currently has two overlapping ideas of context reduction:

- `ContextWindowPolicy`, which can drop messages but does not implement summarization.
- `context_management`, which can generate a `compacted_context` through provider-backed compaction.

The active compaction path is in `context_management`, but its behavior is internally inconsistent. After compaction, the session stores `compacted_context`, resets `messages` to the retained tail, and rebuilds transcript state. However, future provider requests are still assembled from transcript history rather than an explicit `compacted_context + retained tail` model, so semantic continuity can be lost. At the same time, historical tool records can remain provider-visible after compaction, which weakens the token-reduction benefit.

The intended scope of this change is prompt-context correctness and efficiency, not audit retention. Full durable tool history, external security logging, and off-box telemetry are out of scope for this feature.

## Goals / Non-Goals

**Goals:**
- Make future provider requests use `compacted_context + retained tail` as the canonical post-compaction context.
- Ensure compaction materially reduces provider-visible context, including in tool-heavy sessions.
- Allow compaction to destructively prune pre-tail session history, including historical tool records that have been summarized.
- Preserve semantic continuity by requiring important historical tool outcomes to be represented in `compacted_context`.
- Add tests that verify actual request composition and compaction effectiveness, not just local helper behavior.

**Non-Goals:**
- Providing long-term audit history inside the session object.
- Designing or implementing external logging, SIEM export, or compliance pipelines.
- Changing handoff semantics beyond what falls out naturally from the new `compacted_context + retained tail` model.
- Implementing `ContextWindowPolicy::SummarizeAfter` as a second summarization system.

## Decisions

### 1. Post-compaction request context SHALL be explicit rather than inferred from transcript leftovers

Future prompts after compaction will be composed from:

- instructions
- `compacted_context` rendered into provider-visible form
- retained tail only

Rationale:
- This matches the desired product semantics.
- It avoids relying on accidental transcript reconstruction through `timeline` and `tool_records`.
- It makes tests and accounting simpler because the provider-visible context has one clear definition.

Alternatives considered:
- Keep transcript-only reconstruction and inject a synthetic summary message into `messages`: rejected because it keeps summary state coupled to transcript mutation and leaves tool-history retention ambiguous.
- Preserve full transcript plus summary: rejected because it defeats the point of compaction.

### 2. Compaction SHALL be destructive for pre-tail history within the session

When compaction succeeds, the session should discard pre-tail transcript state that has already been summarized. This includes historical tool-call records and timeline entries that are no longer part of the retained tail.

Rationale:
- The user explicitly prefers prompt efficiency over in-session audit durability.
- Keeping old tool history in `timeline` or `tool_records` makes compaction ineffective for tool-heavy sessions.
- A destructive boundary makes it obvious what the provider can still see.

Alternatives considered:
- Keep full `tool_records` for local inspection but omit them from prompt construction: acceptable in principle, but rejected for this feature because it preserves complexity without a current product need.

### 3. Compaction accounting SHALL reflect provider-visible growth, including tool traffic

Compaction triggers should be based on the actual material likely to appear in the next provider request. Tool calls and tool results that are still part of retained history must contribute to compaction accounting and thresholds.

Rationale:
- The current threshold model can miss tool-heavy growth.
- Trigger math should follow the same mental model as prompt assembly.

Alternatives considered:
- Keep the current `uncompacted_tokens` approximation focused on user/assistant text only: rejected because it systematically undercounts many agent sessions.

### 4. Historical tool outcomes SHALL be preserved semantically, not transcriptually

The compaction prompt and parsing model will continue using `CompactedContext` as the semantic memory surface. Historical tool outcomes worth preserving should land in fields like `recent_results`, `established_facts`, `decisions`, `unresolved_questions`, and `notes`.

Rationale:
- Semantic carry-forward is more token efficient than replaying tool logs.
- It avoids duplicating facts in both tool transcript and summary.

Alternatives considered:
- Add a dedicated preserved-tool-log section to `CompactedContext`: rejected for now because it would reintroduce transcript-style baggage.

### 5. Integration tests SHALL verify request composition, not just compaction state

The test suite should assert that after compaction:

- the next provider request contains compacted context and retained tail
- the next provider request excludes pre-tail historical tool traffic
- compaction still triggers for tool-heavy sessions

Rationale:
- Current tests prove helper behavior but do not prove end-to-end correctness.

## Risks / Trade-offs

- [Destructive compaction changes the meaning of `timeline()` and `tool_records()`] -> Document that post-compaction session history reflects retained working state, not full historical audit state.
- [Compacted summaries may omit important tool details] -> Strengthen compaction prompt expectations and add integration tests that cover tool-derived facts/results.
- [Token accounting remains approximate] -> Keep thresholding conservative and validate behavior with tests based on realistic tool traffic.
- [Existing users may confuse `ContextWindowPolicy::SummarizeAfter` with context-management compaction] -> Clarify docs and errors so the supported summarization path is unambiguous.

## Migration Plan

1. Update request construction so provider-visible context is assembled from instructions, compacted context, and retained tail.
2. Update compaction application logic to prune pre-tail transcript state, including historical tool records and timeline entries.
3. Update token accounting so trigger thresholds include tool traffic that remains provider-visible before compaction.
4. Add end-to-end tests covering maintenance compaction, checkpoint compaction, and tool-heavy sessions.
5. Refresh documentation/comments that imply the compacted summary is already part of the next prompt if that is not yet true.

Rollback strategy:
- Revert to the prior transcript-reconstruction approach if regressions emerge, but that would knowingly restore the current prompt-continuity bug.

## Open Questions

- Should the retained tail preserve only transcript messages, or also any tool records directly referenced by retained tail entries?
- Should `ContextWindowPolicy::SummarizeAfter` continue to error, or should docs explicitly steer all summarization use cases to `context_management`?
