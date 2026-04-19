## 1. Result contract and runtime normalization

- [ ] 1.1 Define the v1 plugin result envelope that separates plugin-authored transcript text, optional presentation payload, `presentation_id`, `presentation_mode`, and metadata
- [ ] 1.2 Extend WASM plugin execution result handling to parse, validate, and normalize that envelope
- [ ] 1.3 Preserve compatibility for plain text plugin results by treating them as text-only envelopes

## 2. Client-visible runtime surfaces

- [ ] 2.1 Extend the runtime/facade tool-result event surfaces to carry optional presentation payloads for plugin-backed tool calls
- [ ] 2.2 Ensure plugin-authored transcript text remains present for existing client and logging flows
- [ ] 2.3 Document or codify how presentation payloads are exposed without requiring clients to parse transcript text

## 3. Declarative v1 presentation model

- [ ] 3.1 Define the supported v1 presentation kinds and their schemas (`todo_list`, `status`, `progress`, and any other selected view-oriented kinds)
- [ ] 3.2 Reject or sanitize unsupported executable/arbitrary frontend payloads
- [ ] 3.3 Define the semantics of `presentation_mode` (`replace`, `append`, `transient`) and how clients should interpret them

## 4. Verification

- [ ] 4.1 Add tests for text-only plugin results, rich plugin results, and mixed fallback behavior
- [ ] 4.2 Add tests confirming rich payloads reach client-visible event/facade surfaces intact
- [ ] 4.3 Add tests confirming unsupported/executable UI payloads are rejected or sanitized
- [ ] 4.4 Re-run plugin/runtime regression suites after the new result contract lands
