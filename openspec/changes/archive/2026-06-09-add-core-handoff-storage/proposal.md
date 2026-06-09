## Why

AgentIron currently relies on app-layer JSON file handling for saved handoff bundles even though `iron-core` already owns bundle creation and hydration semantics. Moving saved handoff persistence into `iron-core::config::ConfigStore` gives desktop, CLI, and headless consumers one shared API for saving, loading, listing, deleting, and validating durable handoff snapshots.

## What Changes

- Add typed `ConfigStore` APIs to save, load, list, and delete saved handoff bundles.
- Store saved handoffs as exact serialized `HandoffBundle` snapshots with derived metadata for listing.
- Validate saved handoff identifiers, names, bundle schema versions, and serialized bundle shape at save/load boundaries.
- Add a compiled-in config-store migration for a `saved_handoffs` table.
- Add tests covering save, load, list, delete, update semantics, metadata derivation, and invalid bundle handling.
- Do not strip, sanitize, rewrite, or reinterpret bundle contents during persistence; saved handoffs are sensitive local snapshots.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `core-config-store`: Add durable saved handoff bundle storage APIs and schema requirements to the existing core-owned config store.

## Impact

- Affected public APIs: `iron_core::config::ConfigStore` and related config-store handoff input/record/metadata types.
- Affected storage: SQLite config-store migrations gain a `saved_handoffs` table with metadata columns and authoritative bundle JSON.
- Affected handoff domain: existing `HandoffBundle` serialization and version constants become validation inputs for durable saved handoff records.
- Affected consumers: AgentIron desktop can keep import/export UI while delegating saved handoff storage and validation semantics to `iron-core`; CLI/headless consumers can share the same saved handoff state.
