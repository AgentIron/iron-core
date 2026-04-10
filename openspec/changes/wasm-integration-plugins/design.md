## Context

`iron-core` currently exposes built-in tools and MCP-backed tools, both of which are useful for engineering-oriented agent workflows. AgentIron's next differentiator is a user-friendly integration model for knowledge workers that can connect to SaaS and application ecosystems such as Gmail, Google Calendar, Slack, Discord, GitHub, and Zoom without assuming shell access, external MCP server processes, or API-key-driven setup.

The proposed WASM plugin system introduces a third tool source that is runtime-local like built-in tools, but packageable and isolated like an extension surface. Plugins are expected to run inside the client/runtime boundary, be configured by the client, and interact with users through client-owned UX surfaces for setup and OAuth. The runtime still owns plugin lifecycle, tool exposure, health, and credential storage interfaces.

This change is cross-cutting because it affects runtime inventory, session state, prompt construction, client APIs, security boundaries, and tool execution architecture.

## Goals / Non-Goals

**Goals:**
- Add a runtime-local WASM integration system separate from MCP and built-in tools.
- Make integrations installable from local files or HTTPS URLs, with checksum verification required for remote artifacts.
- Support user-friendly integrations for OAuth- and web-service-driven workflows without requiring direct system access.
- Expose structured plugin metadata and status so diverse clients can present setup and status consistently.
- Standardize on a strict v1 auth model where plugins declare OAuth requirements and `iron-core` governs auth state and tool gating.
- Reuse the existing session-scoped tool visibility pattern so each session can independently enable or disable plugins.
- Keep plugin inventory, auth state, and enablement runtime-local and excluded from handoff.

**Non-Goals:**
- A public marketplace or official registry for unvetted third-party plugins.
- General-purpose trust guarantees for arbitrary third-party plugins beyond checksum verification and capability declaration.
- Rich universal UI definitions shared across Tauri, TUI, and chatbot clients.
- Native/local OS integrations that require broad platform-specific host access.
- Solving multi-account UX in v1.

## Decisions

### Runtime-local integration layer distinct from MCP
The system will treat WASM plugins as a separate integration mechanism rather than an MCP transport variant. MCP remains the interoperability path for external agent/server ecosystems, while WASM plugins target user-friendly, in-process integrations for trusted web services and selected local application scenarios.

Alternatives considered:
- Reuse MCP for all integrations: rejected because the target user experience should not require external servers, npm, shell access, or engineering-oriented deployment assumptions.
- Extend built-in tools directly: rejected because integrations need an isolated packaging and future extension model.

### Remote plugins require checksum validation
Remote plugins loaded from HTTPS URLs must include a valid checksum so the runtime can verify the fetched bytes before execution. Local files remain supported without checksum enforcement, placing trust in the local installation path rather than the remote transport.

Alternatives considered:
- Allow unsigned remote URLs: rejected because mutable remote artifacts make trust too weak even for trusted vendor-hosted plugins.
- Require full signing/version infrastructure in v1: deferred because checksum verification provides a simpler first boundary while keeping room for stronger publisher identity later.

### Plugin metadata must be structured and client-facing
Plugins must declare structured metadata for identity, capabilities, auth requirements, network permissions, exported tools, and user-facing status. `iron-core` will expose these facts to clients, which can render them as GUI controls, TUI text, or chat messages depending on the product surface.

Alternatives considered:
- Tool definitions only: rejected because integrations need setup, warning, and connection status beyond model-facing tool schemas.
- Plugin-defined UI payloads: rejected because cross-client UI portability is a core constraint.

### Session enablement should mirror MCP, but availability must include auth state
Each session will independently enable or disable installed plugins, similar to session-scoped MCP server enablement. Effective plugin tool exposure must also account for plugin runtime health and plugin/tool auth requirements, which means tool visibility is gated by more than session intent.

Alternatives considered:
- Treat plugin auth failures as session disablement: rejected because user intent and auth availability are distinct states.
- Treat plugin availability as all-or-nothing: rejected because some plugins may expose status or read-only tools while write tools remain unavailable.

### Strict v1 auth is manifest-driven and runtime-governed
In v1, plugins may declare auth requirements such as OAuth kind, provider identity, requested scopes, and per-tool scope requirements, but they do not own auth semantics. `iron-core` is authoritative for auth state vocabulary, OAuth lifecycle, credential bindings, and tool availability decisions. Clients provide the user interaction surfaces needed to complete authentication, such as browser launch, redirect capture, code entry, or consent prompts.

Alternatives considered:
- Make clients own all auth semantics: rejected because tool execution and runtime availability would become too fragmented.
- Let plugins own provider auth behavior: rejected because non-technical-user integrations need predictable auth behavior, consistent status reporting, and deterministic tool gating.
- Make `iron-core` own all provider-specific UX: rejected because `iron-core` intentionally supports clients with very different UI stacks.

This strict v1 model intentionally optimizes for first-party OAuth/HTTP integrations. If future integrations require non-standard provider behavior, a narrow extension point can be added later without weakening the initial trust and consistency model.

### Network access must be declared, even when broad access is permitted
Plugins must declare their network policy in metadata. The model must support both allowlisted vendor APIs and broader wildcard access for legitimate cases such as search or generic retrieval, while keeping that authority visible to clients and policy layers.

Alternatives considered:
- Forbid arbitrary HTTP: rejected because it blocks legitimate plugin classes such as web search.
- Allow arbitrary HTTP without declaration: rejected because it hides a major trust boundary.

### Handoff excludes plugin runtime state
Plugin inventory, auth bindings, and session enablement will not travel in handoff bundles. Imported sessions must resolve plugin availability entirely from the destination runtime.

Alternatives considered:
- Include plugin references in handoff: rejected because runtimes may not share plugin artifacts, auth state, or trust policy.

## Risks / Trade-offs

- [OAuth support in `iron-core` may grow provider-specific branches] -> Keep v1 limited to manifest-driven OAuth flows and defer non-standard provider escape hatches until justified by a concrete integration.
- [Checksum verification proves integrity, not trustworthiness] -> Position checksum as a minimum transport integrity control and avoid implying marketplace-style vetting.
- [Wildcard network access weakens isolation] -> Require explicit declaration and surface it in client-visible metadata and policy.
- [Cross-client UX may drift] -> Standardize on machine-readable metadata and action hints rather than portable UI payloads.
- [Plugin capability model may become too generic too early] -> Keep v1 focused on integrations and defer marketplace/no-code concerns.

## Migration Plan

- Introduce the plugin subsystem behind new runtime APIs without changing existing built-in or MCP behavior.
- Add plugin-aware effective tool composition as an additive extension to the current tool surface.
- Keep handoff format unchanged for portable session state, explicitly excluding plugin runtime state.
- Roll back by disabling the plugin subsystem without affecting built-in tools or MCP registries.

## Open Questions

- Should unauthenticated plugins expose any setup/status tools to the model, or should setup remain entirely client-driven?
- What minimum manifest fields are mandatory in v1 versus optional presentation metadata?
- Should the runtime enforce hard deny rules for local/private network targets even when a plugin declares wildcard outbound HTTP?
- How much plugin runtime health should be standardized versus plugin-defined in status reporting?
- What exact client callback payloads are required to support browser-based OAuth flows across GUI, TUI, and bot clients?
