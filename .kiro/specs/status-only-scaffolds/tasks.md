# Tasks

GitHub issue: #9

- [x] Rust-SME approve the exact scaffold package inventory, zero-dependency status-only constraints, and validator boundary.
- [x] Add the 13 status-only core workspace packages with the mandatory paths and authoritative Cargo metadata.
- [x] Add deterministic validator coverage for metadata, paths, dependency prohibition, source/target restrictions, lint inheritance, and Vision-registry alignment.
- [x] Wire the validator into the existing local quality path without provisioning dependencies or changing existing package behavior.
- [x] Run targeted scaffold validation, QA, security, final architecture/Rust-SME, and `make check`.

## Acceptance evidence

- Every added package compiles with only standard library use and exposes no semantic public API.
- Metadata/layout validation rejects drift deterministically.
- QA/security/architecture approve the status-only/no-false-claim boundary.
- Existing packages and runtime behavior remain unchanged.
