## 1. Result contract and runtime normalization

- [x] 1.1 Define the v1 plugin result envelope that separates plugin-authored transcript text, optional `view` payload, `view_id`, `view_mode`, and correlation metadata
- [x] 1.2 Extend WASM plugin execution result handling to parse, validate, and normalize that envelope
- [x] 1.3 Preserve compatibility for plain text plugin results by treating them as text-only envelopes

## 2. Client-visible runtime surfaces

- [x] 2.1 Extend the runtime/facade tool-result event surfaces to carry normalized transcript text and optional `view` payloads for plugin-backed tool calls
- [x] 2.2 Ensure plugin-authored transcript text remains present for existing client and logging flows in ordinary chronological transcript order
- [x] 2.3 Codify how normalized `view` fields are exposed without requiring clients to parse transcript text or reverse-engineer arbitrary JSON

## 3. Declarative v1 view model

- [x] 3.1 Define the supported v1 view kinds and their schemas (`todo_list`, `status`, `progress`)
- [x] 3.2 Reject unknown view kinds and reject executable/arbitrary frontend payloads
- [x] 3.3 Define the semantics of `view_mode` (`replace`, `append`, `transient`) and how clients should interpret them
- [x] 3.4 Define `view_id` scope and reconciliation behavior within a session

## 4. Verification

- [x] 4.1 Add tests for text-only plugin results, rich plugin results, and mixed fallback behavior
- [x] 4.2 Add tests confirming normalized transcript text and `view` payloads reach client-visible event/facade surfaces intact
- [x] 4.3 Add tests confirming unknown or executable rich payloads are rejected
- [x] 4.4 Re-run plugin/runtime regression suites after the new result contract lands
