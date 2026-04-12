## 1. Bootstrap correlation policy

- [x] 1.1 Define and implement the shared acceptance rule for `initialize` responses with null or missing IDs so it applies only in the unambiguous bootstrap case
- [x] 1.2 Preserve strict request/response ID validation for ordinary post-bootstrap MCP requests and explicitly reject or ignore ambiguous id-less responses

## 2. Transport-specific handling

- [x] 2.1 Update the plain HTTP MCP client path to apply the bootstrap exception without weakening normal response validation
- [x] 2.2 Update the stdio MCP reader/dispatcher so id-less bootstrap responses are not dropped before bootstrap correlation is evaluated
- [x] 2.3 Update the HTTP+SSE MCP reader/dispatcher so id-less bootstrap responses are not dropped before bootstrap correlation is evaluated

## 3. Verification

- [x] 3.1 Add transport tests covering successful `initialize` with `id: null` and with an absent `id` for HTTP, stdio, and HTTP+SSE
- [x] 3.2 Add regression tests proving ordinary MCP responses without usable IDs are not accepted once bootstrap is complete or correlation is ambiguous
- [x] 3.3 Run the relevant MCP transport and integration test suites and update any stale expectations about timeout-versus-mismatch failure modes
