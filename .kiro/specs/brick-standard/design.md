# Design

## Taxonomy

```text
capability family
  <brick>          stable domain contract; required only when independently owned
  <brick>-memory        optional deterministic local stateful adapter
  <brick>-<vendor>      optional one-provider / one-integration adapter
  <brick>-mcp           optional bounded control-plane adapter library
  <brick>-mesh          optional peer adapter after a dedicated boundary specification
  <brick>-test-support  optional non-production fixture crate after two consumers
  <brick>-server        optional binary composition root

adapter infrastructure: mcp-transport, cap-std, provider SDKs, databases, network clients
composition bases: MCP server, API server, worker, edge binary
optional domain packs: datasets, ML, security, payments, games, UI, OpenArcade
```

A capability core exposes a stable Rust SDK and ports. Every arrow points inward:

```text
vendor / memory / MCP adapter / server  ──>  capability core
server                                 ──>  MCP adapter + concrete adapters
core                                   ──>  another capability only through its typed port
```

## Mandatory scaffold package shape

A capability family that is committed but not yet designed may use this agent-maintainable status-only core tree as a bounded intermediate step. It is intentionally uniform so agents can enumerate and fill the same responsibility paths deterministically. A family with no demonstrated consumer receives no package at all — it stays a registry row naming its future crate.

```text
crates/<brick>/
  Cargo.toml                       # package.metadata.rust-factory record
  src/
    lib.rs                          # crate-level status and private modules
    model.rs                        # status-only until typed models exist
    validation.rs                   # status-only until rules/bounds exist
    error.rs                        # status-only until error taxonomy exists
    port.rs                         # status-only until a consumed port exists
    service.rs                      # status-only until orchestration exists
  tests/
    public_contract.rs              # status-only until public contract exists
```

A status-only tree has only documentation/comments, compiles, has zero non-stdlib dependencies, exposes no public semantic APIs, and carries no behavior/durability/security claim. Its metadata record is authoritative:

```toml
[package.metadata.rust-factory]
family = "sandbox"
role = "core"
status = "scaffolded"
```

The closed role set is `core`, `memory`, `adapter`, `vendor`, `mcp`, `server`, `mesh`, `infrastructure`, and `test-support`. The closed package-status set is `scaffolded`, `specified`, `implemented`, `migration-pending`, and `deprecated`. Crate-level documentation mirrors those fields; the Vision registry remains the family-level source of truth. `scripts/validate_brick_registry.py` enforces this for every package: it rejects a missing or malformed metadata record, unknown or unregistered families, unknown roles/statuses, a missing or extra status-only path, non-comment status-only source, forbidden status-only dependencies and target/feature configuration, canonical scaffold path mismatches, binary targets under `crates/`, `role = "server"` disagreeing with residence under `projects/`, unlisted or phantom workspace members, package directories without a manifest, and registry disagreement in either direction.

## Mature role shape

A mature capability family may contain only the roles its approved semantics justify:

```text
<brick>          typed models, validation, errors, ports, service
<brick>-memory        process-local adapter for a concrete core-owned stateful port
<brick>-<vendor>      one integration implementing an existing core-owned port
<brick>-mcp           bounded DTO/convert/service library, never process stdio lifecycle
<brick>-server        binary config/composition/runtime/transport/trusted-context owner
<brick>-mesh          native peer adapter only after its own boundary specification
<brick>-test-support  shared non-production fixtures only after two consumers
```

Adapter infrastructure has no core. Composition bases are binaries only. Optional domain packs use the same capability seams but do not introduce domain conditionals into generic cores. A role in a mature shape is not automatically stamped: it appears only when the corresponding contract/topology is approved.

## Boundary contract

```text
untrusted transport
  → framed byte/depth/item ceilings
  → strict serde DTO + schemars discovery schema
  → private conversion to core newtypes/commands
  → core validate_* rules
  → trusted-host context + verified Policy decision
  → injected, bounded effect port
  → safe response DTO + serialization ceiling
  → transport
```

`serde` and `schemars` are the Rust analogue of the DTO/schema portion of Pydantic v2, not a replacement for domain validity. Rust internal models deliberately do not derive transport serialization merely for convenience. Constructors, enums, and `validate_*` functions establish the valid internal state; Policy, catalog availability, tenancy, idempotency, state transitions, and external effects remain runtime/domain checks.

## Existing migration

MCP lifecycle ownership is corrected: the `serve_stdio()` helpers that constructed bounded stdio transports have been deleted from all four `mcp` modules. Those modules retain tool DTOs, routing, safe projections, and a transport-agnostic service entry point; a `role = "server"` binary under `projects/` owns `BoundedStdioTransport` construction and the `serve(...).waiting()` lifecycle. No such binary exists yet, so the transport currently has no production caller (#17).

The next normalization wave closes Project MCP object DTO schemas, makes raw/semantic/egress limits explicit per MCP operation, inventories whether canonical JSON is Workflow domain identity or adapter wire handling, and moves stateful Agent local adapters out of `agent` only if the extraction is proven behavior-preserving and improves the established taxonomy.

## Verification

The future standard’s scaffold acceptance suite includes: core deterministic validation and public-error tests; property tests only for algebraic/canonical invariants; local-adapter persistence/tenant/idempotency/concurrency tests as applicable; MCP malformed/unknown-field/identity-injection/request-response limit tests; Policy pre-effect tests; and black-box server startup/allowed/denied smoke tests. `make check` remains the final workspace gate after every behavior-affecting migration.
