# Tasks

- [x] Obtain formal Rust SME approval for the canonical taxonomy, package roles, ingress/egress sequence, `serde`/`schemars` validation layering, and MCP lifecycle correction.
- [x] Add an approved architecture reference to the Living Factory Vision, README, and scaffolding/project-blueprint guidance; preserve the portfolio tracker as the delivery-status source of truth.
- [x] Revise the canonical standard and Living Factory Vision for mandatory agent-maintainable status-only core scaffolds, the `package.metadata.rust-factory` family/role/status registry, fixed placeholder paths, role eligibility, and the family-level mature-shape taxonomy.
- [x] Deliver the deterministic metadata/layout validator in the first status-only rollout; it rejects unknown roles/statuses, missing required status-only paths, prohibited dependencies, canonical scaffold metadata/path mismatches, and Vision/package scaffold-status drift.
- [ ] Extend metadata/layout validation with semantic contract checks after each family has an independently approved contract, including role-family eligibility and adapter-specific layout rules.
- [ ] Create one focused specification and delivery-gated semantic implementation at a time. Cache, graph, and message-bus are candidates, not a combined batch; start only after each family demonstrates a consumer, stable core-owned port, and independently approved contract.
- [ ] Design a dedicated versioned Project Blueprint successor for canonical family stamping. Project Blueprint V1 remains unchanged and SHALL NOT stamp package families.

## Acceptance evidence

- Formal Rust SME approval resolves the current lifecycle/taxonomy blockers.
- Architecture confirms the standard preserves inward dependencies, thin composition roots, and the Living Factory Vision.
- The standard names no unapproved framework dependency; `serde` and `schemars` remain boundary-only.
- Follow-on lifecycle migrations retain MCP schemas, bounded transport behavior, trusted-context/Policy semantics, and public error projections under their own tests and delivery gates.
