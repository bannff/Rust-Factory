# Tasks

- [x] Add `policy` only to `evaluation::mcp` and replace the legacy trusted-context constructor seam with the verified Policy compatibility resolver.
- [x] Add private bounded stdio JSON-RPC transport with a 64 KiB pre-deserialization inbound frame limit.
- [x] Validate bounded input before trusted-context/Policy access; authorize the exact Evaluation capability before reader/store access.
- [x] Prove source failure, deny, and tampered Allow decision make zero reader/store calls for validate, evaluate, and get.
- [x] Prove invalid/oversized semantic input makes zero source/Policy/reader/store calls.
- [x] Prove allowed evaluate/get retain immutable create-or-match, tenant non-disclosure, read-only Workflow evidence behavior, and safe output projections.
- [x] Prove bounded stdio transport accepts valid fragmented input and terminates oversized frames before dispatch.
- [x] Confirm Evaluation core/memory public contracts and canonical result hash are unchanged; no policy decision is persisted.
- [x] Run `cargo test -p evaluation --features mcp,memory`.
- [x] Run QA, security, Rust SME, architecture gates, and `make check`.

## Acceptance evidence

- Rust SME approved the specification and final implementation.
- QA approved pre-domain authorization, tenant, immutable-result, and transport boundary coverage.
- Security approved the verified resolver and 64 KiB LF/CRLF pre-deserialization transport cap after the CRLF boundary regression fix.
- Architecture approved adapter-only Policy/framework containment and unchanged Evaluation core/memory contracts.
- `make check` passed after the final migration change.

Promotion, experiment execution, durable persistence, and shared MCP transport extraction remain explicitly out of scope.
