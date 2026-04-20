## Context

`iron-core` has a layered prompt composition system but no first-class mechanism for injecting specialized, reusable behavioral guidance. Current approaches require either embedding all expertise in static prompts or relying on plugin infrastructure. Agent Skills provide a lightweight declarative alternative that models can discover, activate on demand, and apply to tasks.

The Agent Skills ecosystem defines `SKILL.md` files with YAML frontmatter and markdown instructions, stored in discoverable directories. The core principle is progressive disclosure: models see a compact catalog at session start, activate skills when relevant, and receive full instructions on demand.

This design must work for:
- Local agents scanning standard filesystem directories
- Cloud/sandboxed clients supplying bundled skills
- Mixed deployments where both sources coexist

## Goals / Non-Goals

**Goals:**
- Unified skill catalog with pluggable discovery sources
- Standard filesystem scanning for `.agents/skills/` directories (project and user scopes)
- Session-scoped skill activation with deduplication
- Dedicated `activate_skill` model tool with progressive disclosure semantics
- Activated skill content protected from context compaction
- Runtime/facade APIs for skill discovery, activation, and inspection
- Trust gating for project-level filesystem skills

**Non-Goals:**
- Persistent skill installation database or package management
- Remote skill registry fetching in core
- Automatic skill selection or recommendation by the model
- Skill-defined code execution hooks
- Skill marketplace or sharing infrastructure
- Plugin-like WASM execution for skills

## Decisions

**1. Skills are prompt-layer primitives, not capabilities or plugins**
- Rationale: Capabilities control runtime access (file system, shell); plugins are executable WASM tools. Skills are instructional overlays. Keeping them separate preserves clear semantic boundaries.
- Alternative considered: Treating skills as a new capability type. Rejected because capabilities are permission-gated runtime affordances, not behavioral guidance.

**2. Source-agnostic discovery via `SkillSource` trait**
- Rationale: Both filesystem scanning and client-provided bundles should feed the same catalog. A trait abstracts this without forcing all skills into files.
- Alternative considered: Filesystem-only with client faking files. Rejected because it creates unnecessary disk I/O and complexity for sandboxed deployments.

**3. Unified catalog with deterministic precedence**
- Rationale: Collisions between project/user/client/built-in skills need predictable resolution.
- Precedence order: trusted project filesystem > client-provided project-scoped > user filesystem > client-provided global > built-in.
- Alternative considered: First-found-wins. Rejected because project-local skills should naturally override broader scopes.

**4. Dedicated `activate_skill` tool as primary activation mechanism**
- Rationale: Cleaner than file-read activation for non-filesystem skills. Gives `iron-core` control over content formatting, resource listing, and structured wrapping.
- Alternative considered: File-read activation only. Rejected because it breaks for bundled/virtual skills and provides no structured wrapping.

**5. Activated skills stored as session state and re-injected into prompts**
- Rationale: Treating activated skills only as historical tool outputs makes them vulnerable to compaction. Explicit session state + re-injection is more robust.
- Alternative considered: Relying on protected tool result messages. Rejected because compaction might still summarize or drop them depending on implementation.

**6. Body-only content returned on activation (frontmatter stripped)**
- Rationale: Frontmatter metadata is already in the catalog. Returning body-only keeps the injected instructions clean and focused.
- Alternative considered: Full `SKILL.md` including frontmatter. Rejected because it adds redundant metadata to the prompt.

**7. Resource listing on activation, but no eager loading**
- Rationale: Progressive disclosure applies to resources too. The model should load referenced files individually using existing file-read tools.
- Alternative considered: Eagerly loading all bundled resources. Rejected because it defeats the token-saving purpose of progressive disclosure.

**8. Malformed filesystem skills fall back to minimal metadata instead of disappearing**
- Rationale: Preserving discoverability with a warning is more operator-friendly than silently removing a skill because of broken frontmatter.
- Alternative considered: Skipping malformed skills entirely. Rejected because it hides recoverable content and makes broken skill directories harder to diagnose.

**9. Additional configured skill directories are trusted user-level configuration**
- Rationale: `additional_skill_dirs` are explicit runtime configuration, so they should not inherit repository trust gating meant for scanned project content.
- Alternative considered: Treating configured directories like project skills. Rejected because admin-configured paths are an intentional trust decision and should not be suppressed by repository trust settings.

**10. `requires_trust` only gates model-initiated activation**
- Rationale: Facade/runtime APIs are called by the embedding application or user-controlled code, not by the model, so they should remain available for explicit activation flows.
- Alternative considered: Enforcing `requires_trust` everywhere. Rejected because it would block legitimate host-side control paths without adding meaningful model safety.

**11. No baseline prompt hint for inactive skills in v1**
- Rationale: The dynamic `activate_skill` tool already exposes the catalog when skills are available. Adding a generic baseline hint would consume tokens on every request whether or not skills matter.
- Alternative considered: Always adding a small prompt hint when skills are available. Rejected for now because it adds prompt noise without evidence that tool discoverability is insufficient.

## Risks / Trade-offs

- **Large skill catalogs inflate tool descriptions** → Mitigation: Catalog is compact (name + description only, ~50-100 tokens per skill). Cap or paginate if needed.
- **Activated skill content competes for context window** → Mitigation: Progressive disclosure limits loaded skills to those actually used. Body size should be capped (recommended <5000 tokens).
- **Project skills from untrusted repositories** → Mitigation: Trust gate blocks project skills unless project is marked trusted. Log diagnostics for transparency.
- **Skill catalog refresh timing** → Mitigation: Refresh on session creation by default. Later could support per-turn refresh for dynamic environments.
- **Compaction protection complexity** → Mitigation: Activated skills stored in explicit session state and re-injected as a dedicated prompt layer. This is simpler than trying to mark arbitrary historical messages as protected.

## Migration Plan

No migration needed. This is an additive feature:
- Existing sessions without skills behave identically
- No breaking changes to APIs, session formats, or tool contracts
- Clients opt in by registering skill sources or using filesystem scanning

## Open Questions

1. Should bundled/client skill resources be readable in v1, or only listed?
2. Should skill catalog refresh happen per session, per turn, or on explicit request after the initial session snapshot?
3. Should `iron-core` support skill deactivation (removing from session), or is session lifecycle sufficient?
4. Should activated skills be visible in the runtime context display alongside capabilities?
