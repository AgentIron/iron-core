## Context

The MCP protocol structs in `src/mcp/protocol.rs` currently use Rust snake_case field names directly for serde serialization and deserialization. MCP wire messages require camelCase field names. This means outbound requests such as `initialize` are encoded incorrectly (`protocol_version`, `client_info`), and inbound responses from spec-conforming servers may also fail to deserialize (`serverInfo`, `nextCursor`, `inputSchema`, `isError`, `mimeType`).

This is a protocol-shape correctness bug affecting all transports because stdio, HTTP, and HTTP+SSE all share the same message definitions. Real servers including filesystem, firecrawl, and context7 are failing, indicating the issue is systemic rather than server-specific.

## Goals / Non-Goals

**Goals:**
- Make MCP request serialization conform to the MCP camelCase wire format.
- Make MCP response deserialization accept the MCP camelCase wire format.
- Audit all existing MCP protocol structs used on the wire so the fix is complete, not limited only to `initialize`.
- Add tests that prove the serialized JSON shape and response parsing behavior.

**Non-Goals:**
- Changing MCP transport architecture, retries, or connection lifecycle policy.
- Adding backward-compatibility support for snake_case server responses unless already provided implicitly by serde aliases.
- Changing non-MCP internal Rust naming conventions; Rust field names remain snake_case in code.

## Decisions

### Apply serde camelCase mapping at the MCP protocol struct boundary
Use `#[serde(rename_all = "camelCase")]` on the relevant structs in `protocol::messages` so Rust code remains idiomatic snake_case while the wire format becomes MCP-compliant.

This is preferable to per-field `#[serde(rename = ...)]` annotations because the protocol structs follow a consistent naming convention and `rename_all` is less error-prone to maintain.

### Audit all wire-visible MCP message structs, not just initialize
The fix must include both request and response structs used on the MCP wire, including initialize, tool list, tool call, and content/resource payloads where fields differ between snake_case and camelCase.

This is preferable to patching only `InitializeRequest` because partial correction would leave the client non-compliant in later protocol stages.

### Add explicit serialization/deserialization tests for protocol shapes
Tests should assert JSON output for outbound messages and JSON parsing for inbound responses using the expected camelCase field names.

This is preferable to relying only on end-to-end integration tests because protocol shape bugs are easiest to diagnose with targeted serialization tests.

### Keep Rust field names unchanged
Retain snake_case Rust field names in code and treat camelCase as a wire-format concern only.

This is preferable to renaming Rust fields to camelCase because it would be unidiomatic in Rust and would spread protocol concerns into internal naming.

## Risks / Trade-offs

- [A struct might be missed during the audit] → Add targeted tests covering initialize, list tools, call tool, and content/resource parsing so omissions are caught quickly.
- [Some non-conforming servers may currently rely on snake_case] → The project should prefer MCP specification compliance; if compatibility problems appear later, handle them explicitly as a separate interop decision.
- [Changing deserialization may reveal additional protocol mismatches after initialize succeeds] → Treat this as useful signal; fixing camelCase may expose the next real protocol bug rather than masking it.

## Migration Plan

- Add camelCase serde mapping to all relevant MCP protocol structs.
- Add focused unit tests for serialization and deserialization shape.
- Run MCP integration tests against existing fake transports.
- Re-test against real failing servers (filesystem, firecrawl, context7) after implementation.

## Open Questions

- Do we want to accept both camelCase and snake_case on inbound deserialization using serde aliases for extra interoperability, or should we enforce camelCase-only wire support for now?
