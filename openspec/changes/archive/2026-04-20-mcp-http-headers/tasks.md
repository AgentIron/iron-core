## 1. Data Model

- [x] 1.1 Add `HttpConfig` struct to `src/mcp/server.rs` with `url: String` and `headers: Option<HashMap<String, String>>`, with `#[serde(default)]` on headers and `#[serde(skip_serializing_if = "Option::is_none")]` to omit when unset
- [x] 1.2 Change `McpTransport::Http` variant from `{ url: String }` to `{ #[serde(flatten)] config: HttpConfig }`
- [x] 1.3 Change `McpTransport::HttpSse` variant from `{ url: String }` to `{ #[serde(flatten)] config: HttpConfig }`
- [x] 1.4 Verify `McpTransport` still derives `Debug, Clone, Serialize, Deserialize, PartialEq, Eq` and compiles

## 2. HTTP Client — HttpMcpClient

- [x] 2.1 Update `HttpMcpClient::new()` to accept `headers: Option<HashMap<String, String>>` and store them
- [x] 2.2 Add a helper method or inline logic in `send_request()` that builds the request with the default `Accept: application/json, text/event-stream` header, then overlays user-configured headers (user headers win on conflict)
- [x] 2.3 Verify `HttpMcpClient` compiles and the trait impl is satisfied

## 3. HTTP Client — HttpSseMcpClient

- [x] 3.1 Update `HttpSseMcpClient::new()` to accept `headers: Option<HashMap<String, String>>` and store them
- [x] 3.2 Add the `Accept` header and custom headers to the SSE bootstrap GET request in `ensure_sse_reader()` (the spawned task needs access to headers — clone an `Arc<HashMap>` or owned copy)
- [x] 3.3 Add the `Accept` header and custom headers to the JSON-RPC POST request in `send_request()`
- [x] 3.4 Verify `HttpSseMcpClient` compiles and the trait impl is satisfied

## 4. Dispatch Wiring

- [x] 4.1 Update `create_transport_client()` to extract `HttpConfig` from the matched variant and pass `url` and `headers` to `HttpMcpClient::new()` and `HttpSseMcpClient::new()`

## 5. Test Updates

- [x] 5.1 Update all `McpTransport::Http { url: ... }` constructions in test files to `McpTransport::Http { config: HttpConfig { url: ..., headers: None } }` (or use `HttpConfig::new(url)` if a constructor is added)
- [x] 5.2 Update all `McpTransport::HttpSse { url: ... }` constructions in test files similarly
- [x] 5.3 Add a unit test verifying `HttpConfig` with `headers: None` serializes without a `headers` key
- [x] 5.4 Add a unit test verifying `HttpConfig` with custom headers serializes and deserializes round-trip correctly
- [x] 5.5 Add an integration test verifying HTTP requests include the `Accept` header (can use a mock server that asserts on incoming headers)
- [x] 5.6 Add an integration test verifying custom headers are sent alongside the default `Accept` header

## 6. Verification

- [x] 6.1 Run `cargo check` to confirm the full crate compiles
- [x] 6.2 Run `cargo test` to confirm all existing and new tests pass
