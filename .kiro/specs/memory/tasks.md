# Memory Tasks

Tracked by [#18](https://github.com/bannff/Rust-Factory/issues/18).

- [x] 1. Survey `agentic-memory` 0.4.2's real public API, dependency weight, and async posture before designing anything against it.
- [x] 2. Design gate on the port and adapter shape. `rust-factory-sme` returned BLOCKED with 4 blockers and 11 corrections; the user overrode the block and directed implementation. Recorded in requirements §7.1 with the accepted risk and the mitigations actually in place.
- [x] 3. Implement the framework-free core: `model`, `validation`, `error`, `port`, `service`. Every limit is the brick's own constant, never inherited from a vendor.
- [x] 4. Implement the `local` adapter as the reference behaviour for the port contract, and the `agentic` adapter with structural isolation — one graph per `(tenant, namespace)`.
- [x] 5. Implement `settings` for declarative backend selection, owning configuration shape only.
- [x] 6. Public contract, service contract, and shared adapter conformance tests. Every adapter runs the same eight-clause suite plus a cross-adapter agreement test.
- [x] 7. QA gate. Three real bugs fixed: a failed write destroyed the previous record and poisoned the key; a query drove the vendor's degrading per-key lookup and was quadratic; per-request limits bounded one call and nothing else.
- [x] 8. Security gate. Fixed capacity ceilings, a result-ceiling bypass through the port, defence in depth beyond the tenant, and a `Debug` impl that reprinted the variant the projection exists to hide.
- [x] 9. Final `rust-factory-sme` and `meta-architect` gates on the capability. Moved the capacity rule out of an adapter, nested both stores by tenant, corrected an overstated validation guarantee, and made read-your-writes a stated precondition.
- [x] 10. Add the `mcp` module: five tools, one `CapabilityV1` variant each, authorization on capability **and** `memory_enabled`, transport ceilings before the gate and semantics after.
- [x] 11. QA and security gates on the MCP surface. Fixed mutually inconsistent ceilings that made `search` fail permanently after six records, a raw-length content bound that escaping defeated, and refusal indistinguishable from backend failure.
- [x] 12. Final gates on the MCP surface. `meta-architect` found five governing documents asserting the opposite of the code; all corrected.
- [x] 13. Update the Vision registry row, README, brick standard, adapter portfolio, framework policy, and `policy`'s contract matrix.
- [x] 14. Recovered combined-tree evidence: focused QA/security/final Rust SME/final architecture gates approved, full `make check` passed, `memory` has 90 all-feature tests and 25 framework-free default tests.
- [x] 15. Commit the recovered combined Memory/Observability tree after focused recovery gates and a successful full `make check` (`f702dc2`).

## Deferred, each separately gated

- [ ] A durable adapter. Requires its own specification for leases, recovery, cross-process cancellation, and exactly-once effects. **Precondition:** an audit seam must exist first, because `memory_forget` is an irreversible unaudited delete whose blast radius is currently one process lifetime only because both adapters are in-process (gap 11).
- [ ] Migrate `agent`'s provisional `MemoryStore` port onto this one. A one-way migration; `agent` currently carries a `recall`/`write` port against a `Vec<String>` stub, so a project composing both has two memory ports in scope (gap 2).
- [ ] Prove declarative selection end to end. The `settings` module owns configuration shape; the backend-to-constructor `match` belongs to the first composition binary, which does not exist (gap 3).
- [ ] Principal scoping and per-principal capacity. Capacity is a tenant-shared budget today, so one principal can starve its peers (gaps 7 and 10). A quota changes the isolation model.
- [ ] A `MemoryQuery` cursor. `deferred_keys` names what a page could not carry but there is no resumable position (gap 13). When it lands, `deferred_keys` should be re-derived from the cursor rather than kept as a second model of partiality.
- [ ] Reconcile refusal projection across the four MCP surfaces — [#20](https://github.com/bannff/Rust-Factory/issues/20).
- [ ] Add a supply-chain gate for the new pin — [#24](https://github.com/bannff/Rust-Factory/issues/24).
