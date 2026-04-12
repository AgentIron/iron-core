## MODIFIED Requirements

### Requirement: Runtime supports concrete MCP transport clients
The runtime SHALL provide concrete transport support for configured MCP servers using the declared transport type, including stdio, HTTP, and HTTP+SSE. For stdio transports, the runtime SHALL spawn the subprocess with the parent process environment minus environment variables whose names match sensitive credential patterns, rather than a hardcoded allowlist. The runtime SHALL strip vars matching case-insensitive suffix patterns associated with secrets (`_API_KEY`, `_SECRET`, `_SECRET_KEY`, `_TOKEN`, `_PASSWORD`, `_CREDENTIALS`, `_AUTH_TOKEN`, `_ACCESS_KEY`, `_ACCESS_TOKEN`) and well-known credential var names (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AZURE_CLIENT_SECRET`, `GOOGLE_APPLICATION_CREDENTIALS`, `DATABASE_URL`, `GITHUB_TOKEN`, `GH_TOKEN`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). The runtime SHALL log the names of stripped vars at debug level without logging their values. User-configured env vars from the MCP server config SHALL be merged after stripping and SHALL override any stripped or inherited values.

#### Scenario: Stdio subprocess inherits non-sensitive parent environment vars
- **WHEN** a configured MCP server uses stdio transport and the parent process has environment variables that do not match sensitive patterns
- **THEN** the spawned subprocess inherits those non-sensitive vars
- **THEN** common toolchain vars like `PATH`, `HOME`, `APPDATA`, `XDG_CONFIG_HOME`, `CARGO_HOME`, `GOPATH`, `NODE_PATH` are available to the subprocess without requiring explicit MCP server config

#### Scenario: Stdio subprocess strips vars matching sensitive suffix patterns
- **WHEN** the parent process has environment variables whose names end in `_API_KEY`, `_SECRET`, `_TOKEN`, `_PASSWORD`, or similar sensitive suffixes
- **THEN** those vars are not present in the spawned subprocess environment
- **THEN** the runtime logs the names of stripped vars at debug level

#### Scenario: Stdio subprocess strips well-known credential vars
- **WHEN** the parent process has environment variables like `AWS_ACCESS_KEY_ID`, `GITHUB_TOKEN`, `ANTHROPIC_API_KEY`, or other well-known credential names
- **THEN** those vars are not present in the spawned subprocess environment

#### Scenario: User-configured env overrides stripped vars
- **WHEN** an MCP server config specifies an env var that would otherwise be stripped by the sensitive pattern matching
- **THEN** the user-configured value is present in the subprocess environment
- **THEN** the user config acts as an explicit override

#### Scenario: Sensitive pattern matching is case-insensitive
- **WHEN** the parent process has an environment variable whose name matches a sensitive pattern with different casing (e.g., `My_Api_Key` matching `_API_KEY`)
- **THEN** that var is still stripped
