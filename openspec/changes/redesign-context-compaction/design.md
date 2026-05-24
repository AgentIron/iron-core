## Context

Context compaction currently happens as a hidden runtime behavior. The runtime estimates context pressure, splits the session into older history plus retained tail, sends the older history to a separate summarizer call, parses a structured `CompactedContext`, stores that structure, and later prepends it to future provider requests.

That design has several problems: the summarizer is not the active model that understands the current task, the structured JSON parser is brittle, compaction is invisible to the model and user, range choice is runtime-imposed rather than semantically informed, and long sessions accumulate pressure from tool traffic and repeated summaries. The product is still in testing mode, so this change can break old compacted sessions and remove `CompactedContext` without migration.

## Goals / Non-Goals

**Goals:**

- Replace hidden runtime compaction with an explicit model-visible `compress` tool for tool-capable models.
- Store all compacted history as chronological freeform compressed blocks.
- Remove `CompactedContext` and the separate hidden summarizer execution path.
- Preserve tool transcript integrity by validating compression ranges against structural groups.
- Allow previous completed turns and older compressed blocks to be compacted during an active turn without compressing the active turn.
- Expose qualitative context-pressure nudges to the model while keeping exact telemetry internal.
- Make `/compact` immediately run a compression-focused model turn.
- Keep the first implementation strict and predictable: reject invalid ranges, do not auto-adjust, and do not add end-user nudge UI.

**Non-Goals:**

- Supporting non-tool-calling models with hidden fallback compaction.
- Migrating old persisted `CompactedContext` sessions.
- Adding semantic protected-content categories beyond structural protections.
- Adding hierarchy, aging, or omission policy for old compressed blocks beyond allowing future summary-of-summary compression.
- Fixing token counting accuracy, which is tracked separately.
- Redesigning model-switch behavior.

## Decisions

### Use Freeform Compressed Blocks as the Only Durable Representation

Compression output will be stored as durable freeform blocks with metadata such as block ID, topic, source range, summary, creation time, and optional before/after telemetry. Existing structured `CompactedContext` fields will be removed.

Rationale: the active model can write the most useful durable summary directly, and requiring a structured schema reintroduces parsing brittleness and creates ambiguity between structured and freeform memory.

Alternative considered: keep `CompactedContext` for fallback while adding freeform blocks for `compress`. This was rejected because it creates two authoritative compaction models and complicates rendering, merging, and future compression.

### Make `compress` Runtime-Owned and Special

The `compress` tool will be exposed in the effective tool catalog when compression is available, but execution will be intercepted by the runtime rather than routed through MCP, plugin, Python, approval, or child-tool dispatch.

Rationale: compression mutates durable session structure and must apply strict internal safety checks. Treating it like a normal external tool risks incorrect approval behavior and makes state mutation harder to reason about.

Alternative considered: implement `compress` as an ordinary built-in registry tool. This was rejected because the tool needs privileged access to timeline IDs, pressure state, and destructive session pruning.

### Address Ranges with Stable Visible IDs, Validate with Structural Groups

Durable timeline entries will have stable internal IDs. When compression is available or useful, provider-visible transcript rendering will include synthetic IDs for user, assistant, tool, and compressed-block entries. Compressed blocks will use IDs such as `c0003` and can participate in later compression ranges.

The model may reference visible IDs, but validation will operate on structural groups. Tool call/result pairs, pending or running tool operations, the latest user request, and the current assistant turn cannot be split or compressed incorrectly.

Rationale: visible IDs give the model precise anchors, while structural validation prevents stranded tool records or incoherent transcript state.

Alternative considered: expose only safe grouped units. This was rejected for the initial design because it reduces precision and obscures how the visible timeline maps to provider-visible context.

### Remove Active Transcript Entries and Store Blocks Separately

When a range is compressed, selected active transcript entries and selected older compressed blocks are removed from provider-visible active history and replaced by one or more chronological compressed blocks stored separately. The main timeline will not receive synthetic replacement transcript entries.

Rationale: synthetic replacement entries would make very long sessions increasingly artificial and could distort the one-large-timeline mental model. Separate chronological blocks keep active history clean while preserving durable summaries.

Alternative considered: insert replacement entries at the original range position. This was rejected because repeated compaction over long sessions would create nested pseudo-turns that are hard for future models to interpret.

### Render Compressed Blocks Chronologically by Compaction Time

Future provider requests will render compressed blocks in creation order before the retained active transcript, using a consistent block format that includes the block ID, topic, source range, and summary.

Rationale: chronological rendering matches the user experience of one long session and avoids topic regrouping that could reorder causality.

Alternative considered: group blocks by topic or render newest first. This was rejected because it can obscure the order in which decisions and constraints emerged.

### Use Qualitative Pressure Buckets in Prompts

The runtime will compute exact context telemetry internally, but the system prompt will render only qualitative pressure buckets such as `none`, `soft`, `medium`, `strong`, or `critical`, plus action guidance. Prompt cache invalidation should be based on bucket changes, not exact token estimates.

Rationale: exact percentages are noisy, can invalidate prompt caches frequently, and may be inaccurate until token counting improves. Buckets are easier for the model to follow and for tests to assert.

Alternative considered: expose exact token usage and remaining window. This was rejected for prompt stability and because current estimates are heuristic.

### Clear Pressure Only After Recomputed Usage Falls Below Threshold

A valid `compress` call mutates state, but the pressure nudge only clears when recomputed provider-visible context usage falls below the active threshold. If compression succeeds but usage remains high, the tool result should tell the model more compression may be needed.

Rationale: the invariant is context fit, not the raw number of messages or tokens compressed.

Alternative considered: require a minimum range size or token count before accepting compression. This was rejected because small compressions can still be valid and pressure state should be based on recomputed context usage.

### Make `/compact` an Immediate Compression Turn

When the user invokes `/compact`, the runtime will immediately run a model turn whose task is to compress resolved context using the `compress` tool. It will inject a strong compression nudge rather than running hidden runtime compaction.

Rationale: users expect the command to take immediate action, and using the same model-visible path keeps all compaction behavior consistent.

Alternative considered: attach a strong nudge to the next normal prompt. This was rejected because the command would appear inert.

### Fail Simply at Critical Pressure

If context pressure is critical and compression fails validation, fails to execute, or succeeds without getting usage under the required threshold after the allowed attempt, the runtime will surface a simple user-visible error recommending a new session.

Rationale: indefinite compression loops are risky and confusing. A clear failure preserves control and avoids hidden destructive behavior.

Alternative considered: repeatedly ask the model to compress until under threshold. This was rejected for the first implementation because it can loop, increase cost, and still fail near hard limits.

## Risks / Trade-offs

- Freeform summaries may omit important details -> The compress tool prompt must state that the summary permanently replaces the selected range and must preserve durable facts, decisions, constraints, file paths, errors, and user intent.
- Visible IDs add prompt overhead -> Render IDs only when compression is available or useful, and keep ID syntax compact.
- Range validation can reject reasonable model choices -> Return precise validation errors and require the model to retry with safe boundaries rather than auto-adjusting silently.
- Removing `CompactedContext` breaks old compacted sessions -> Accept this while in testing mode; no migration path is required.
- No non-tool fallback means some providers may lack compaction -> Treat model-driven compaction as requiring tool calling; provider/model capability changes are out of scope for this change.
- Pressure buckets depend on imperfect token estimates -> Keep exact values internal and bucket behavior conservative; accurate token counting remains a separate tracked effort.
