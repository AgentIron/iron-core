## 1. Protocol struct audit and correction

- [x] 1.1 Audit all MCP wire-visible structs in `src/mcp/protocol.rs` and identify fields that require camelCase serde mapping
- [x] 1.2 Add `#[serde(rename_all = "camelCase")]` (or precise field aliases where needed) to the relevant MCP request/response/content structs

## 2. Verification

- [x] 2.1 Add unit tests that serialize outbound MCP requests and assert camelCase JSON field names for initialize and other relevant request payloads
- [x] 2.2 Add unit tests that deserialize camelCase MCP responses for initialize, tools/list, tools/call, and resource/content payloads
- [x] 2.3 Run MCP transport and integration tests and update any expectations that depended on the incorrect snake_case wire format

## 3. Real-world validation

- [x] 3.1 Re-test MCP connectivity against the known failing servers (filesystem, firecrawl, context7) or add documented follow-up notes if any remaining failures are due to separate issues
