## Context

`iron-core` already compiles successfully against `iron-providers` `0.1.1`, but the manifest and user-facing setup docs still reference `0.1.0`. The crate also keeps `monty` on a git dependency because crates.io currently only provides a placeholder package rather than a usable library release.

## Goals / Non-Goals

**Goals:**
- Align the declared `iron-providers` dependency version with the current supported release.
- Keep README and getting-started examples consistent with the manifest.
- Preserve the existing Monty policy and describe it accurately.

**Non-Goals:**
- Changing any `iron-core` runtime behavior.
- Pinning or otherwise changing the `monty` dependency source.
- Introducing broader dependency management automation.

## Decisions

Update `iron-providers` from `0.1.0` to `0.1.1` in the crate manifest and all documented dependency examples.
Rationale: the repository already builds against `0.1.1`, so the change is declarative and keeps published guidance current.

Keep `monty` on `branch = "main"`.
Rationale: the user explicitly chose to defer any Monty dependency policy change until a usable crates.io release exists.

Refresh nearby explanatory comments only where they are now stale.
Rationale: this keeps the change minimal while avoiding documentation that implies Monty has no upstream releases at all.

## Risks / Trade-offs

- [Docs drift again after future provider releases] -> Keep version references centralized and limited to the manifest plus setup docs.
- [Monty main changes underneath future builds] -> Accepted for now because the project intentionally tracks git main until crates.io is viable.
