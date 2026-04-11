## 1. Runtime Wiring

- [x] 1.1 Store a single runtime-owned `McpConnectionManager` inside `IronRuntime` and start it when MCP is enabled
- [x] 1.2 Update `register_mcp_server()` so runtime registration schedules immediate connection/discovery through that shared manager
- [x] 1.3 Decouple runtime connection state from `enabled_by_default` so per-session defaults do not suppress runtime discovery

## 2. Effective Tool Surface

- [x] 2.1 Rework `SessionToolCatalog` to reference real runtime-owned local tool executors instead of cloned `ToolRegistry` state
- [x] 2.2 Switch prompt construction in `PromptRunner` to `build_inference_request_with_effective_tools()` using the session-effective tool definitions
- [x] 2.3 Preserve normal local tool execution through the same session-effective path and add regression coverage for existing ACP tool-call behavior

## 3. Embedded Python

- [x] 3.1 Replace the legacy `ToolCatalog::from_registry(...)` path in `execute_python_script()` with a session-effective tool snapshot
- [x] 3.2 Route Python child-tool lookup, approval checks, validation, and execution through `SessionToolCatalog`
- [x] 3.3 Ensure `cargo check --features embedded-python` passes and add feature-enabled regression coverage

## 4. Transport and Execution

- [x] 4.1 Implement true transport handling for all declared MCP transports, including real HTTP+SSE support rather than aliasing it to plain HTTP
- [x] 4.2 Verify model-issued and child-issued MCP tool calls both use the shared runtime connection manager and durable lifecycle
- [x] 4.3 Harden unavailable/disconnected/error-state failures with tests that exercise real connection-manager behavior instead of only registry mutation

## 5. Verification

- [x] 5.1 Add end-to-end tests that register MCP servers, drive real connection/init/discovery, and assert prompt-visible tool exposure
- [x] 5.2 Add end-to-end execution tests that exercise real MCP tool calls through the runtime, including reconnect and rediscovery behavior
- [x] 5.3 Re-run existing ACP/runtime tool-call tests and embedded-Python tests to confirm MCP changes do not regress local tool behavior
