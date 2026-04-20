## 1. Auth interaction model

- [x] 1.1 Define the runtime-owned auth prompt/status schema exposed to clients
- [x] 1.2 Define the direct client auth-start/auth-complete flow and API surface
- [x] 1.3 Define how auth state transitions are surfaced to the rest of the session

## 2. Runtime/client surfaces

- [x] 2.1 Expose runtime-owned auth interaction state through inspection and/or event APIs
- [x] 2.2 Allow clients to trigger auth flows directly without model mediation
- [x] 2.3 Recompute and expose plugin availability after auth state changes

## 3. Verification

- [x] 3.1 Add tests for unauthenticated, pending, and authenticated client-visible auth states
- [x] 3.2 Add tests for direct client-started auth flow behavior
- [x] 3.3 Add tests confirming plugin availability updates after auth transitions
