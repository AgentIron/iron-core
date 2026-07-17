## Why

`iron-core` exposes a large public Rust API, but strict rustdoc currently reports more than one thousand undocumented public items, its examples are ignored rather than compiled, and its README contains stale installation, version, release, and documentation guidance. Current API documentation should be complete, validated on every pull request, and published from `main` so consumers can rely on it before and after releases.

## What Changes

- Document every currently public module and public API item without narrowing visibility or exempting existing public surfaces from missing-documentation enforcement.
- Add compiling rustdoc examples for primary usage paths and compile README Rust snippets as doctests, using hidden scaffolding or `no_run` only where execution would cause external side effects.
- Repair broken rustdoc links and fail documentation validation on rustdoc warnings or missing public documentation.
- Add a local `inv docs` task and run it in pull request CI without deploying documentation.
- Publish rustdoc output for the current `main` branch through the repository's existing GitHub Pages configuration and custom domain.
- Perform a detailed README review covering installation, supported Rust and dependency versions, public API positioning, examples, feature claims, development commands, release behavior, documentation links, and GitHub Pages usage.
- Preserve the existing public API; public-surface reduction remains a possible follow-up after the generated documentation can be reviewed.

## Capabilities

### New Capabilities

- `automated-documentation`: Complete and strictly validated public API documentation, compiling examples and README guidance, pull request documentation checks, and GitHub Pages publication from `main`.

### Modified Capabilities

None.

## Impact

- Rustdoc comments and examples throughout `src/` for all currently public modules and items.
- Crate-level documentation and README doctest inclusion in `src/lib.rs`.
- README installation, development, release, feature, and documentation guidance.
- Local Invoke tasks in `tasks.py`.
- Pull request CI and a new GitHub Pages deployment workflow under `.github/workflows/`.
- GitHub Pages output at the existing `core.agentiron.ai` custom domain.
- No intended runtime behavior, public signature, dependency, or visibility changes.
