## 1. Request Composition

- [x] 1.1 Refactor provider request construction so post-compaction prompts are assembled from instructions, `compacted_context`, and retained tail.
- [x] 1.2 Remove any remaining request-path dependence on historical pre-tail transcript reconstruction after compaction.

## 2. Destructive Compaction State

- [x] 2.1 Update compaction application logic to prune pre-tail transcript state from `messages`, `timeline`, and historical tool-call records.
- [x] 2.2 Ensure retained tail state remains internally consistent after destructive pruning.

## 3. Trigger Accounting

- [x] 3.1 Update compaction accounting so tool calls and tool results contribute to provider-visible context growth before compaction.
- [x] 3.2 Verify maintenance and hard-fit compaction decisions use the corrected accounting model.

## 4. Verification

- [x] 4.1 Add integration tests that assert future requests include compacted context and retained tail after compaction.
- [x] 4.2 Add integration tests that assert pre-tail historical tool transcript is absent from future requests after compaction.
- [x] 4.3 Add integration tests covering tool-heavy sessions that should trigger compaction.

## 5. Documentation

- [x] 5.1 Update comments and user-facing docs to describe `compacted_context + retained tail` as the supported post-compaction prompt model.
- [x] 5.2 Clarify the relationship between `context_management` compaction and `ContextWindowPolicy::SummarizeAfter` so supported behavior is unambiguous.
