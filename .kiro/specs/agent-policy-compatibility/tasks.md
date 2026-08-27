# Tasks

- [x] Add adapter-only `policy-core`, exact bounded-transport dependencies, and private 64 KiB stdio transport to `agent-mcp`.
- [x] Replace unprotected Agent MCP construction with verified host-context/Policy resolver construction.
- [x] Validate bounded inputs before source/Policy access and authorize the exact capability before every domain path.
- [x] Prove all five MCP schemas reject each caller-supplied identity/policy field—including zero-argument list—before trusted context, Policy, or domain access.
- [x] Prove source, context-conversion, canonicalization, and tampered Allow evidence map exactly to adapter-local `operation_failed`; deny maps exactly to `not_found`; both make zero domain-port calls for all five operations.
- [x] Prove malformed/oversized definition, ID, and invocation input make zero source/Policy/domain calls.
- [x] Invoke via canonical verified grant → `EffectiveCapabilityCeilingV1` → `invoke_with_ceiling`; prove denied tools/capabilities are absent from model scope and ports.
- [x] Prove globally shared definition behavior remains explicit and Policy does not create false tenant-private claims.
- [x] Prove bounded LF/CRLF transport framing, exact limit, fragmented oversize terminal behavior, successor suppression, and cancellation-safe partial input.
- [x] Confirm `agent-core` public contracts, dependency graph, and direct full-scope `invoke` compatibility remain unchanged.
- [x] Run `cargo test -p agent-core -p agent-mcp`, QA, security, Rust SME, architecture, and `make check`.

## Acceptance evidence

- Rust SME approved the specification and final implementation.
- QA approved the five-operation schema/port matrix, runtime ceiling composition, global V1 definition behavior, and final ingress regression.
- Security approved trusted-context provenance, request-bound decision verification, pre-effect ceiling enforcement, and bounded ingress.
- Architecture approved the adapter-only dependency direction and Policy-free Agent core after this checklist reconciliation.
- `make check` passed after the final ingress-boundary correction.

Agent definition tenancy, durable workflow controls, shared transport extraction, experiments, and promotion remain out of scope.
