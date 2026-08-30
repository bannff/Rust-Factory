# Design

## Taxonomy

```text
capability family
  crates/<brick>                one capability crate (`role = "brick"`)
    typed core                  models, validation, errors, ports, service
    feature-gated adapters      mcp, memory/local, fs, settings, or one vendor module
  crates/<brick>-mesh           optional peer adapter after a dedicated boundary specification
  crates/<brick>-test-support   optional non-production fixtures after two consumers

status-only family
  crates/<brick>                bounded intermediate package (`role = "core"`)

adapter infrastructure          shared package owning no capability (`role = "infrastructure"`)
composition base
  projects/<name>               binary composition root (`role = "server"`)
optional domain pack            uses the same capability seams
```

A capability crate exposes a stable Rust SDK and ports. Its adapters are modules, not packages. Every dependency points inward:

```text
feature-gated adapter module  ──>  capability core
server                       ──>  brick adapter modules + concrete implementations
core                         ──>  another capability only through its typed port
```

The capability roadmap and taxonomy live in GitHub issues and GitHub Projects. Local package metadata describes packages that exist; it is not a roadmap and is not cross-checked against external roadmap membership.

## Mandatory scaffold package shape

A capability family that is committed but not yet designed may use this agent-maintainable status-only core tree as a bounded intermediate step. It is intentionally uniform so agents can enumerate and fill the same responsibility paths deterministically. A family with no demonstrated consumer receives no package at all and remains roadmap-only in its GitHub issue and GitHub Projects entry.

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

A status-only tree has only documentation/comments, compiles, has zero non-stdlib dependencies, exposes no public semantic APIs, and carries no behavior/durability/security claim. Its metadata is the authoritative machine-readable record for that package only:

```toml
[package.metadata.rust-factory]
family = "sandbox"
role = "core"
status = "scaffolded"
```

The closed role set is `brick`, `core`, `infrastructure`, `server`, `mesh`, and `test-support`. The closed package-status set is `scaffolded`, `specified`, `implemented`, `migration-pending`, and `deprecated`. Crate-level documentation mirrors those fields. `scripts/validate_brick_registry.py` enforces local workspace/package structure: workspace inventory; package naming, placement, targets, and exactly-three-field metadata; status-only family placement, shape, source content, and configuration; feature defaults; adapter dependency and module isolation for bricks; and Makefile coverage for every brick and declared feature. It accepts any non-empty family name and does not validate GitHub issue or GitHub Projects membership, Vision drift, or bidirectional roadmap agreement.

## Mature role shape

A mature capability family keeps its typed core and eligible adapters in one brick crate:

```text
crates/<brick>/
  src/{model,validation,error,port,service}.rs   typed core
  src/mcp.rs or src/mcp/                         bounded control-plane adapter
  src/{memory,local,fs,settings}.rs              eligible local/config adapters
  src/<vendor>.rs                                one vendor integration named for its crate

projects/<name>/                                 optional server composition root
crates/<brick>-mesh/                             optional peer adapter package
crates/<brick>-test-support/                     optional shared-fixture package
```

Every adapter module is feature-gated, including adapters with no dependency, and no feature is enabled by default. `mcp`, `memory`, `adapter`, and `vendor` are retired as package roles; adapters do not receive separate package metadata. Only `server`, `mesh`, and `test-support` are separate roles for artifacts a brick cannot contain. Shared cross-family infrastructure uses `infrastructure` and owns no capability. Composition bases are binaries only. Optional domain packs use the same capability seams but do not introduce domain conditionals into generic cores. A module or separate role appears only when its corresponding contract or topology is approved.

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
