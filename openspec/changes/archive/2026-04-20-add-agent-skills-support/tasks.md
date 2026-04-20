## 1. Core types and skill source abstractions

- [x] 1.1 Define `SkillMetadata`, `SkillOrigin`, `SkillLocation`, `LoadedSkill`, `SkillResourceEntry` types
- [x] 1.2 Define `SkillSource` trait with `discover()` and `load()` methods
- [x] 1.3 Implement `FilesystemSkillSource` for standard directory scanning
- [x] 1.4 Implement `StaticSkillSource` for client-provided/bundled skills
- [x] 1.5 Add `SkillCatalog` struct with unified storage and collision resolution
- [x] 1.6 Implement deterministic precedence logic (project > client > user > built-in)

## 2. Skill discovery and parsing

- [ ] 2.1 Scan `<project>/.agents/skills/` and `~/.agents/skills/` directories
- [ ] 2.2 Parse `SKILL.md` frontmatter (YAML) and body (markdown)
- [ ] 2.3 Handle malformed YAML gracefully with fallback parsing
- [ ] 2.4 Extract `name`, `description`, and compute `location` from filesystem paths
- [ ] 2.5 Record diagnostics for skipped/invalid skills without blocking discovery
- [ ] 2.6 Add trust gate for project-level skills (hide unless project trusted)

## 3. Runtime integration and APIs

- [ ] 3.1 Add skill source registration API to `IronRuntime`
- [ ] 3.2 Add `refresh_skill_catalog()` lifecycle method
- [ ] 3.3 Add `list_available_skills()` runtime API
- [ ] 3.4 Add `activate_skill(session_id, name)` runtime API
- [ ] 3.5 Add `list_active_skills()` runtime API
- [ ] 3.6 Expose skill APIs through facade (`AgentSession` methods)

## 4. Session activation state

- [ ] 4.1 Add `ActivatedSkillRecord` and `SessionSkillState` to durable session
- [ ] 4.2 Implement skill activation with deduplication
- [ ] 4.3 Implement skill deactivation (optional, for session reset)
- [ ] 4.4 Ensure activated skills are session-scoped and do not leak across sessions
- [ ] 4.5 Add `is_skill_active(name)` helper for session queries

## 5. Model-facing activation tool

- [ ] 5.1 Define `activate_skill` tool schema with enum-constrained `skill_name` parameter
- [ ] 5.2 Build compact skill catalog for tool description (name + description only)
- [ ] 5.3 Return skill body wrapped in `<skill_content>` structured tags
- [ ] 5.4 Include resource listing in activation result
- [ ] 5.5 Handle invalid skill names with clear error responses
- [ ] 5.6 Omit tool registration when no skills are available

## 6. Prompt integration

- [ ] 6.1 Extend `PromptCompositionConfig` to include active skill instructions layer
- [ ] 6.2 Modify `build_composed_instructions()` to inject activated skill content
- [ ] 6.3 Add minimal behavior hint to baseline prompt when skills are available
- [ ] 6.4 Ensure skill instructions appear in correct prompt ordering
- [ ] 6.5 Include active skills in runtime context display (optional)

## 7. Context compaction protection

- [ ] 7.1 Mark activated skill content as protected from compaction
- [ ] 7.2 Ensure re-injected skill instructions survive hard-fit compaction
- [ ] 7.3 Ensure re-injected skill instructions survive maintenance compaction
- [ ] 7.4 Verify skills are not summarized into `compacted_context`
- [ ] 7.5 Add compaction tests covering skill preservation

## 8. Verification and tests

- [ ] 8.1 Test filesystem discovery with multiple scopes
- [ ] 8.2 Test collision resolution and precedence rules
- [ ] 8.3 Test trust gating for project skills
- [ ] 8.4 Test client-provided skill registration and activation
- [ ] 8.5 Test activation deduplication and idempotency
- [ ] 8.6 Test invalid skill name rejection
- [ ] 8.7 Test prompt injection of active skills
- [ ] 8.8 Test compaction protection for activated skills
- [ ] 8.9 Test session isolation (skills do not leak across sessions)
- [ ] 8.10 Test empty catalog behavior (no tool registered)
- [ ] 8.11 Test malformed skill file handling
- [ ] 8.12 Run full regression suite after implementation
