# context-token-accounting Specification

## Purpose
TBD - created by archiving change api-reported-token-accounting. Update Purpose after archive.
## Requirements
### Requirement: Provider usage establishes context baseline
The system SHALL record provider-reported token usage from `ProviderEvent::Usage` and use reported input tokens as the preferred baseline for provider-visible context estimates.

#### Scenario: Usage event records baseline
- **WHEN** a provider emits a usage event containing `input_tokens`
- **THEN** the session token tracker records those input tokens as the current provider-reported baseline
- **AND** future context estimates use that baseline while it remains valid

#### Scenario: Usage without input tokens does not establish baseline
- **WHEN** a provider emits a usage event without `input_tokens`
- **THEN** the session token tracker records any available output or cache usage for accumulated usage telemetry
- **AND** it does not replace the current context baseline with an unknown input value

### Requirement: Context estimates add local delta after baseline
The system SHALL estimate current context usage as the last valid provider-reported input-token baseline plus locally estimated provider-visible additions after that baseline.

#### Scenario: New user message adds delta
- **WHEN** a usage baseline exists
- **AND** a new user message is appended after that baseline
- **THEN** the current context estimate includes the baseline input tokens plus an estimate for the new user message

#### Scenario: Assistant response remains delta for next request
- **WHEN** a provider reports `input_tokens` for a request
- **AND** the same request produces assistant text or assistant tool calls
- **THEN** the assistant response content is counted as delta for the next provider request
- **AND** the usage baseline is not treated as including that response content

#### Scenario: Tool result adds delta
- **WHEN** a usage baseline exists
- **AND** a tool result is appended after an assistant tool call
- **THEN** the current context estimate includes an estimate for the tool result as local delta

### Requirement: Heuristic fallback remains available
The system SHALL fall back to full provider-visible heuristic estimation when no valid provider-reported baseline is available.

#### Scenario: First call uses heuristic estimate
- **WHEN** a session has not received any provider usage with input tokens
- **THEN** active context telemetry estimates the full provider-visible context using the existing heuristic
- **AND** the telemetry quality remains estimated

#### Scenario: Usage-less provider keeps heuristic behavior
- **WHEN** a provider never emits usage events
- **THEN** context pressure and active context telemetry continue using heuristic estimates
- **AND** the session remains usable without provider usage support

### Requirement: Baseline invalidates on provider-visible rewrite
The system SHALL invalidate the provider-reported input-token baseline when provider-visible context is rewritten rather than append-only.

#### Scenario: Compaction invalidates baseline
- **WHEN** compaction removes historical provider-visible context and creates compressed blocks
- **THEN** the session token tracker clears the provider-reported input-token baseline
- **AND** subsequent context estimates use full heuristic fallback until the next usage-bearing response establishes a new baseline

#### Scenario: Request envelope rewrite invalidates baseline
- **WHEN** instructions, active skill instructions, tool definitions, workspace-root prompt content, or compressed block rendering changes outside append-only transcript updates
- **THEN** the session token tracker invalidates or conservatively refreshes the context estimate before using it for pressure decisions

### Requirement: Accumulated usage uses provider values only
The system SHALL accumulate input, output, cached-input, cache-creation, cache-read, and reasoning-output token totals from provider-reported usage values only.

#### Scenario: Usage event increments totals
- **WHEN** a provider emits usage containing input and output token counts
- **THEN** the session token tracker adds those counts to accumulated usage totals
- **AND** those totals are available to telemetry consumers

#### Scenario: Heuristic delta does not increment cost totals
- **WHEN** local heuristic delta estimation is used for context pressure
- **THEN** the estimated delta is not added to accumulated provider usage totals

### Requirement: Active context telemetry reports best available accounting
The system SHALL expose active context telemetry using the best available accounting source for the session.

#### Scenario: Baseline plus delta telemetry
- **WHEN** a valid usage baseline exists and local delta has been estimated
- **THEN** active context telemetry reports total tokens as baseline plus delta
- **AND** categories or quality metadata indicate that the value is not a fully provider-exact count when local delta is present

#### Scenario: Provider resync updates telemetry
- **WHEN** a later provider response reports new input tokens
- **THEN** subsequent active context telemetry uses the new input-token baseline
- **AND** does not continue accumulating stale pre-resync delta

