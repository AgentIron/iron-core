# agent-skills Specification

## Purpose
Define how the runtime discovers, exposes, activates, and preserves Agent Skills across sessions and compaction.

## Requirements
### Requirement: Skills SHALL be discoverable through pluggable sources
The runtime SHALL support multiple skill discovery sources and normalize discovered skills into a unified catalog.

#### Scenario: Filesystem source discovers project skills
- **WHEN** a `FilesystemSkillSource` scans `<project>/.agents/skills/`
- **THEN** it discovers skill directories containing `SKILL.md` files
- **AND** it extracts `name`, `description`, and location metadata from each skill's frontmatter

#### Scenario: Filesystem source discovers user skills
- **WHEN** a `FilesystemSkillSource` scans `~/.agents/skills/`
- **THEN** it discovers skill directories containing `SKILL.md` files
- **AND** it extracts metadata consistent with the project scope

#### Scenario: Client-provided source registers bundled skills
- **WHEN** a client registers a `StaticSkillSource` with skill metadata and body content
- **THEN** those skills appear in the unified catalog alongside filesystem-discovered skills
- **AND** they are indistinguishable from filesystem skills in the model-facing catalog

#### Scenario: Discovery handles malformed skill files gracefully
- **WHEN** a skill directory contains a malformed or unparseable `SKILL.md`
- **THEN** that skill remains discoverable with fallback metadata derived from the skill directory
- **AND** a diagnostic is recorded without blocking other skill discovery

### Requirement: The unified skill catalog SHALL resolve collisions with deterministic precedence
When multiple sources discover skills with the same `name`, the runtime SHALL apply a deterministic precedence rule.

#### Scenario: Project skill overrides user skill
- **WHEN** both project and user scopes contain a skill named `code-review`
- **THEN** the project-scoped skill is visible in the catalog
- **AND** the user-scoped skill is shadowed

#### Scenario: Collision produces a diagnostic
- **WHEN** a skill collision occurs
- **THEN** the runtime records a diagnostic noting which skill was shadowed

### Requirement: Project-level filesystem skills SHALL be trust-gated
The runtime SHALL not include project-level filesystem skills in the catalog unless the project is marked as trusted.

#### Scenario: Untrusted project skills are hidden
- **WHEN** a project contains skills in `.agents/skills/`
- **AND** the project is not marked as trusted
- **THEN** those skills do not appear in the available catalog
- **AND** a trust diagnostic is recorded

#### Scenario: Trusted project skills are visible
- **WHEN** a project contains skills in `.agents/skills/`
- **AND** the project is marked as trusted
- **THEN** those skills appear normally in the available catalog

#### Scenario: Additional configured skill directories are trusted
- **WHEN** the runtime is configured with `additional_skill_dirs`
- **THEN** skills discovered from those directories appear in the catalog without project trust gating
- **AND** they behave like trusted user-scoped configuration

### Requirement: The model SHALL see a compact available-skills catalog
The runtime SHALL expose discovered skills to the model through a compact catalog that includes only `name` and `description`.

#### Scenario: Catalog is embedded in the activation tool description
- **WHEN** skills are available for the session
- **THEN** the `activate_skill` tool description includes a catalog of available skill names and descriptions
- **AND** the catalog omits skills the user has disabled or that fail trust checks

#### Scenario: Empty catalog omits skill tooling entirely
- **WHEN** no skills are available for the session
- **THEN** the `activate_skill` tool is not registered
- **AND** no empty skill catalog appears in prompts

### Requirement: Skills SHALL be activated through a dedicated model tool
The runtime SHALL provide an `activate_skill` tool that loads a skill's full instructions into the session context.

#### Scenario: Model activates a skill by name
- **WHEN** the model calls `activate_skill` with a valid skill name from the catalog
- **THEN** the skill's instructions are loaded into the session
- **AND** the model receives the skill body wrapped in structured tags

#### Scenario: Duplicate activation is idempotent
- **WHEN** the model calls `activate_skill` for a skill that is already active in the session
- **THEN** the skill is not re-injected
- **AND** the model receives a lightweight confirmation

#### Scenario: Invalid skill name is rejected
- **WHEN** the model calls `activate_skill` with a name not in the catalog
- **THEN** the tool returns an error indicating the skill is not available

#### Scenario: Client activation may bypass model trust gating
- **WHEN** a client calls the facade activation API for a `requires_trust` skill
- **THEN** the skill may be activated for that session
- **AND** only model-initiated activation remains trust-gated

### Requirement: Activated skill content SHALL be protected from context compaction
The runtime SHALL ensure that activated skill instructions survive context compaction for the duration of the session.

#### Scenario: Activated skills survive compaction
- **WHEN** a session undergoes context compaction
- **AND** skills have been activated in that session
- **THEN** the activated skill instructions are preserved in subsequent provider requests
- **AND** they are not summarized or dropped

#### Scenario: Protected skill content is re-injected into composed prompts
- **WHEN** a prompt is composed for a session with activated skills
- **THEN** the activated skill instructions appear as a dedicated instruction layer
- **AND** they participate in normal prompt ordering alongside baseline and session instructions

### Requirement: Activated skills SHALL carry resource listings without eager loading
When a skill is activated, the runtime SHALL enumerate bundled resources but SHALL NOT eagerly load their contents.

#### Scenario: Activation returns resource listing
- **WHEN** a skill with bundled resources is activated
- **THEN** the activation result includes a list of available resource paths
- **AND** the model loads specific resources on demand using existing file-read tools

#### Scenario: Virtual skill resources are listed
- **WHEN** a client-provided skill with virtual resources is activated
- **THEN** the activation result includes resource paths relative to the skill's virtual root

### Requirement: Skill activation SHALL be session-scoped
Activated skills SHALL be tracked per-session and SHALL NOT leak across sessions.

#### Scenario: Active skills are session-local
- **WHEN** skills are activated in session A
- **THEN** session B does not see those activations
- **AND** session B has its own independent catalog snapshot

#### Scenario: Session exposes active skill list
- **WHEN** a client queries a session for active skills
- **THEN** the session returns the names of currently activated skills
