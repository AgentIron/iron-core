## MODIFIED Requirements

### Requirement: Client-visible MCP inspection and session control APIs
The system SHALL provide client-visible APIs to inspect runtime MCP inventory and to enable or disable configured MCP servers for a session. The system SHALL expose enough state for clients to distinguish runtime server health from session enablement intent. When the runtime renders prompt/runtime context for a model request, the displayed working directory and workspace roots SHALL reflect the configured builtin tool roots when such roots are available, rather than the process current directory alone.

#### Scenario: Prompt runtime context uses configured primary root as working directory
- **WHEN** builtin tool roots are configured
- **THEN** the runtime context displays the first configured root as the working directory

#### Scenario: Prompt runtime context exposes all configured roots as workspace roots
- **WHEN** multiple builtin tool roots are configured
- **THEN** the runtime context includes all configured roots in its workspace root list

#### Scenario: Prompt runtime context falls back to process current directory when no roots are configured
- **WHEN** no builtin tool roots are configured
- **THEN** the runtime context uses the process current directory as the displayed working directory
