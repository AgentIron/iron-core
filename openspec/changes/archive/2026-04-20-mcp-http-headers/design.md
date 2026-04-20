## Context

The MCP HTTP transport layer in `src/mcp/client.rs` uses `reqwest::Client::new()` with no default headers. Two client structs — `HttpMcpClient` and `HttpSseMcpClient` — make HTTP requests carrying only the implicit `Content-Type: application/json` from `.json()`. Remote MCP servers like Context7 require `Accept: application/json, text/event-stream` and reject requests without it, surfacing as a misleading "Response ID mismatch" error in iron-core.

The `McpTransport` enum in `src/mcp/server.rs` has `Http { url: String }` and `HttpSse { url: String }` variants with no header fields. The dispatch function `create_transport_client()` passes only the URL to client constructors.

## Goals / Non-Goals

**Goals:**
- Send `Accept: application/json, text/event-stream` on all MCP HTTP requests by default
- Allow per-server custom headers (e.g., `Authorization`, `CONTEXT7_API_KEY`) to be configured and sent
- Introduce a shared `HttpConfig` struct to avoid duplicating header fields across `Http` and `HttpSse` variants
- Maintain backward compatibility for serde deserialization (existing configs without headers continue to work)

**Non-Goals:**
- Changing the stdio transport or its environment variable handling
- Adding header validation or sanitization beyond what reqwest provides
- Supporting per-request header overrides (headers are per-server, set at config time)
- Changing the SSE event parsing logic

## Decisions

### 1. Shared `HttpConfig` struct over duplicating fields on enum variants

**Decision:** Create `HttpConfig { url: String, headers: Option<HashMap<String, String>> }` and use it in both `McpTransport::Http { config: HttpConfig }` and `McpTransport::HttpSse { config: HttpConfig }`.

**Rationale:** Both HTTP variants need the same fields. A shared struct avoids duplication and makes it easy to add future HTTP-specific config (timeouts, TLS settings) in one place. The alternative — duplicating `headers` on each variant — would drift over time.

**Alternative considered:** Putting `headers` on `McpServerConfig` instead. Rejected because headers are transport-specific (stdio doesn't use them) and the struct already has a clear transport field.

### 2. `Option<HashMap>` for headers rather than empty-vec default

**Decision:** Use `headers: Option<HashMap<String, String>>` with serde `#[serde(default)]`.

**Rationale:** `Option` clearly distinguishes "no headers configured" from "empty headers map". With `#[serde(default)]`, existing TOML/JSON configs that don't include `headers` deserialize correctly to `None`.

### 3. Default `Accept` header set at request time, not on the `reqwest::Client`

**Decision:** Add the `Accept` header via `.header()` on each request builder, merging with any user-configured headers. Do not set it as a default header on `reqwest::Client`.

**Rationale:** Setting it per-request makes the behavior explicit and auditable in code. It also allows user-configured headers to override the default `Accept` if needed (e.g., for non-standard servers). The `reqwest::Client` builder approach would silently merge and make overrides harder to reason about.

### 4. Merge order: default Accept first, then user headers

**Decision:** Apply the default `Accept` header first, then overlay user-configured headers. If a user explicitly sets `Accept`, their value wins.

**Rationale:** Users should be able to override defaults for edge-case servers. The merge order is predictable and documented.

## Risks / Trade-offs

- **[Breaking API change]** → `McpTransport::Http` and `HttpSse` variant shapes change. Downstream code constructing these must update. Mitigated by clear compiler errors (Rust enum variants are exhaustive) and the scope being limited to transport construction sites.
- **[Serde compatibility]** → Existing serialized configs use `{ "url": "..." }` which won't match `{ "config": { "url": "..." } }`. Mitigated by using `#[serde(flatten)]` on the `config` field so `Http { url, headers }` still deserializes from `{ "url": "...", "headers": { ... } }` at the variant level — no nesting change.
- **[Header injection]** → User-configured headers are passed through verbatim to reqwest. No sanitization. This is acceptable because the config is trusted (set by the application, not end-user input in a browser).
