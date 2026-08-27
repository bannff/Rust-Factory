---
inclusion: always
---
# Living Factory Vision

## North star

Rust Factory is a domain-agnostic, agent-operated factory for authoring, validating, deploying, and evolving reliable Rust projects and autonomous agents. An agent must be able to discover, compose, and drive bounded Factory capabilities through MCP, while deployed systems remain portable, efficient, and safe Rust programs.

> MCP controls and composes capabilities; Rust executes them locally; mesh protocols coordinate deployed peers.

This document is a living architectural north star. Update it as validated implementation evidence changes the design.

## One capability, multiple surfaces

A brick has a transport-independent Rust core and may expose several adapters:

```text
one brick crate
  ├── mcp module        agent authoring and operational control   (feature)
  ├── typed Rust API    embedded or local Rust callers            (always)
  └── mesh adapter      offline and peer-to-peer coordination     (separate crate)
```

The core owns typed domain models, rules, validation, and stable traits. Adapters own MCP, storage, model-provider, network, and framework details. Dependencies flow inward: adapters depend on core; core never depends on adapters.

MCP is the Factory control plane, not the universal runtime API. Every brick SHOULD provide a bounded MCP surface for discovery and agent operation, but embedded applications SHOULD invoke the typed Rust API directly. Mesh communication SHOULD use a protocol selected for peer discovery, latency, offline operation, and device constraints—not an MCP round trip.

## Composition and deployment topology

MCP adapters are libraries. A thin binary composition root owns process topology: Tokio startup, transport binding, configuration, host-derived trusted context, Policy resolver construction, concrete adapter injection, and orderly shutdown. Cores and MCP libraries do not choose a process topology.

A consuming core calls another capability through the consumed capability's typed port. Local and edge composition inject direct Rust implementations. A remote client is an opt-in adapter that implements that same consumed port only after a dedicated process-boundary specification proves trusted context is independently derived at the receiver and defines authorization, request/result ceilings, idempotency, deadline/cancellation propagation, evidence, and honest recovery guarantees. MCP client calls are never hardcoded in core orchestration.

A linked edge binary is a local composition topology, not a mesh adapter. It inherits the guarantees of its selected local adapters and makes no mesh, durable recovery, cross-process cancellation, or distributed Policy propagation claim without separately specified adapters.

## Three planes

1. **Authoring plane — MCP.** Agents discover bricks and use bounded tools to create projects, configure agents, retrieve steering/skills/knowledge, operate workflows, and inspect evaluation evidence.
2. **Execution plane — native Rust.** A deployed agent uses typed ports such as `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, `Sandbox`, and `ModelProvider`, all currently owned by `agent`. Concrete implementations remain replaceable adapters. A `MessageBus` port is intended but does not exist; see the Message bus row in the portfolio registry.
3. **Mesh/data plane — native peer protocol.** Deployed peers discover, communicate, and replicate explicitly selected data while remaining capable of offline operation and recovery.

## Agentic control-plane ownership

- **Agents** decide, interpret, plan, and compose capabilities within explicit policy and tool scopes.
- **Bricks** execute typed, bounded operations and guarantee their contracts.
- **Workflow** owns durable lifecycle: attempt state, budgets, retries, cancellation, recovery, and terminal reasons.
- **Evaluation** independently assesses evidence and acceptance criteria.
- **Provider and framework adapters** connect models, sandboxes, storage, networks, and transports without defining the Factory core.

Agent definitions are data: identity, model policy, instructions, skills, steering, permitted tools, memory/knowledge policy, sandbox policy, and communication policy. Domain variation is data and composition, never `if domain == ...` orchestration branches.

## Mesh and CRDT safety

Use CRDTs for eventually consistent replicated state—capability advertisements, configuration, observations, indexes, and selected knowledge. Do not rely on CRDT convergence alone for consequential side effects. Commands with external effects require explicit identity, authorization, idempotency, acknowledgement, audit evidence, and, when needed, leases, quorum, or a leader rule.

## Brick anatomy

A mature brick may be split into:

```text
crates/
  <brick>/src/
    lib.rs           crate docs, feature-gated mod declarations, re-exports
    model.rs         domain models
    validation.rs    rules and invariants
    error.rs         stable error taxonomy
    port.rs          consumed effect traits
    service.rs       orchestration
    mcp.rs           agent-facing MCP surface        #[cfg(feature = "mcp")]
    memory.rs        deterministic local adapter     #[cfg(feature = "memory")]
    fs.rs            filesystem adapter              #[cfg(feature = "fs")]
  mcp-transport      shared bounded MCP transport, owns no capability
projects/
  <name>             thin binary composition root, one per deployable
```

A brick is one crate; a module is one adapter. Only three roles justify a
separate package, because a brick cannot contain them: a deployable binary
(`server`, under `projects/`), a peer-coordination adapter (`mesh`), and shared
test fixtures (`test-support`).

A composition root is named for the deployable it produces, not for a capability family: it owns no capability and may host several brick MCP surfaces. The per-family `-server` shape recorded in the registry's mature-shape column is the single-capability case of that same rule.

Not every brick needs every adapter. Start with the smallest set that proves a real capability. Avoid bespoke abstractions where the standard library or a mature framework already provides the required primitive.

The Rust-SME-approved [Canonical Brick Standard](../specs/brick-standard/requirements.md) is the mandatory scaffold contract for new or refactored portfolio entries. It defines role eligibility, crate/test layout, inward dependency direction, strict boundary DTO conversion, explicit domain validation, policy-before-effect behavior, safe bounded egress, and binary-owned process lifecycle. It is intentionally Rust-native: `serde` and `schemars` describe bounded transport/configuration DTOs; validated Rust types and core rules own domain validity. The shared transport migration is complete and no library owns process lifecycle: `serve_stdio` has been removed from every brick, so transport binding belongs solely to a `projects/` binary. No such binary exists yet, so the workspace currently produces libraries only.

## Brick portfolio registry

GitHub [Issue #11](https://github.com/bannff/Rust-Factory/issues/11) tracks the current taxonomy; it supersedes the scaffold-breadth portion of [Issue #9](https://github.com/bannff/Rust-Factory/issues/9). This table is the family-level source of truth for taxonomy, owning crate, intended mature shape, and implementation state.

**A registry row is the parking space, not an empty crate.** Every capability family has a named owning crate here before any code is written, so a new concern always has an unambiguous home and never accumulates inside an unrelated brick. A family receives an actual package only when the flagship autonomous loop—or another demonstrated consumer—drives it. A row whose state is `Deferred` names its future crate and has no package on disk; creating that package early would freeze a contract that no consumer has yet shaped.

A **status-only** package is the narrow intermediate step for a family that is committed but not yet designed:

```text
crates/<family>/
  Cargo.toml                         # [package.metadata.rust-factory]
  src/{lib.rs,model.rs,validation.rs,error.rs,port.rs,service.rs}
  tests/public_contract.rs
```

It has `family`, `role = "core"`, and `status = "scaffolded"` in Cargo metadata; contains only compile-safe status documentation/comments; and has no non-stdlib dependencies, public semantic API, or guarantee claim. When a family becomes real, its existing paths gain the typed model, validation, error, port, service, and contract-test contents—agents never need to guess where a concern belongs.

Every package in the workspace declares `family`, `role`, and `status` in `[package.metadata.rust-factory]`. The closed roles are `brick` (a capability crate with its feature-gated adapter modules), `core` (a status-only family with no behavior yet), `infrastructure` (shared, owns no capability), and the three a brick cannot contain: `server`, `mesh`, and `test-support`. The closed statuses are `scaffolded`, `specified`, `implemented`, `migration-pending`, and `deprecated`. Of the five, only `scaffolded` carries an enforced structural obligation; the rest are declarations of intent and are not guarantees.

`scripts/validate_brick_registry.py` enforces this registry through `make check` in **both** directions: no package may declare a family absent from this table, and no capability family in this table may go undeclared. The validator holds its own declared family list deliberately — it is the independent second statement that this table is checked against, so collapsing the two into one source would make the cross-check tautological and let a bad table edit self-authorize. Adding or retiring a family means editing both, and they must agree.

| Family | Taxonomy | Owning crate | Mature shape when justified | Current state |
|---|---|---|---|---|
| Project authoring | Capability | `project` | `project` with `fs` and `mcp` modules, plus a `projects/` binary | Implemented; MCP tools unauthenticated (#15) |
| Policy / authorization | Capability | `policy` | `policy` with `memory` and durable/provider resolver modules; no MCP by design | Implemented; process-boundary semantics pending (#5) |
| Agent | Capability | `agent` | `agent` with model/tool/memory/knowledge/sandbox adapter modules and `mcp` | Implemented; local server #6 pending |
| Workflow | Capability | `workflow` | `workflow` with `memory`, durable adapter, and `mcp` modules | Implemented; durable adapter pending |
| Evaluation | Capability | `evaluation` | `evaluation` with `memory`, evaluator adapter, and `mcp` modules | Implemented; evaluator portfolio pending |
| Model gateway | Capability | `model-gateway` | `model-gateway` with provider adapter and `mcp` modules | Scaffolded; provisional `ModelProvider` port stays in `agent` until extraction is separately gated |
| Memory | Capability | `memory` | `memory` with local/durable/index adapter and `mcp` modules | Scaffolded; provisional `MemoryStore` port stays in `agent` until extraction is separately gated |
| Sandbox | Capability | `sandbox` | `sandbox` with deny/local/confined adapter modules covering isolated tool and test execution with captured evidence, and `mcp` | Scaffolded; provisional `Sandbox` port, `DenySandbox`, and the `ToolRegistry` port stay in `agent` |
| Observability / audit | Capability | `observability` | `observability` with logging/tracing/metrics/audit adapter and `mcp` modules | Scaffolded; no port yet — `agent::InvocationEvent` is returned in band, not published |
| Workspace governance | Capability | `workspace-governance` | `workspace-governance` with Cargo/governance adapter and `mcp` modules | Deferred; the `make check` validator covers this ground for now |
| Identity / authentication | Capability | `identity` | `identity` with trusted-host/provider adapter and `mcp` modules | Deferred; process-boundary Policy (#5) is a prerequisite and local single-process operation needs no principal authentication |
| Knowledge | Capability | `knowledge` | `knowledge` with local/index/vector/graph adapter and `mcp` modules | Deferred; the `KnowledgeStore` port and `StaticKnowledgeStore` remain owned by `agent` |
| Verification | Capability | `verification` | `verification` with attestation/provenance/reproducibility adapter and `mcp` modules | Deferred; a live-fact check is not a capability — Evaluation owns acceptance and Observability owns evidence. Justified only by signed attestation, provenance chains, or reproducible-artifact proof across multiple consumers |
| Message bus / events | Capability | `message-bus` | `message-bus` with local/durable/broker adapter and `mcp` modules | Deferred; no `MessageBus` port exists and a separate semantic spec is required |
| Cache | Capability | `cache` | `cache` with local/Redis-like adapter and `mcp` modules | Deferred; separate semantic spec required. Distinct from storage: a cache may lose everything at restart without being wrong |
| Graph / provenance | Capability | `graph` | `graph` with local/database adapter and `mcp` modules | Deferred; separate semantic spec required |
| Notification | Capability | `notification` | `notification` with local/provider adapter and `mcp` modules | Deferred; separate semantic spec required |
| Configuration | Adapter infrastructure | No capability crate; server config modules | bounded config-source adapters owned by composition binaries | Agent server config specified |
| Storage | Adapter infrastructure | No generic storage crate | capability-owned persistence ports and adapters | Local adapters exist; generic facade prohibited |
| HTTP / integrations | Adapter infrastructure | No capability crate | consumed-port remote/integration adapters | Deferred pending #5 |
| Browser automation | Optional capability/domain pack | `browser` only when an approved automation contract exists | `browser` with bounded browser adapter and `mcp` modules | Optional; not scaffolded |
| MCP transport | Adapter infrastructure | `mcp-transport` | bounded server transport adapter plus shared registration, schema, and error-projection helpers that keep each `-mcp` crate thin | Implemented; lifecycle migration pending in consumers |
| MCP / API / worker / edge | Composition bases | No core; one binary crate per deployable under `projects/<name>/` | selected binary roots plus config/composition modules | Agent server #6 specified; `projects/` not yet created |
| Data / learning | Optional domain packs | Portfolio registry only until a concrete product contract exists | dataset/ML/learning capability families | Deferred |
| Security, payments, blockchain, games, UI, OpenArcade | Optional domain packs | Portfolio registry only | product-specific composition from generic seams | Excluded from generic core contracts |

A mature role is instantiated only after its own eligibility is met: `-memory` after a concrete stateful core port; `-adapter`/`-<vendor>` to implement an existing port; `-mcp` after a bounded operational surface; `-server` after topology/configuration is specified; and `-mesh` only after a dedicated process-boundary specification. Existing packages migrate behavior-preservingly to this metadata/layout standard. Project Blueprint V1 remains a single-package generator and SHALL NOT stamp family scaffolds; a separately approved successor will do so.

Libraries live under `crates/` and never declare a binary target. Deployable binaries live under `projects/<name>/`, carry `role = "server"`, and are the only packages that own a process: Tokio startup, transport binding, configuration, host-derived trusted context, Policy resolver construction, concrete adapter injection, and orderly shutdown. A project composes only the bricks it needs—there is no aggregate binary that must host every capability.

**A brick is one crate.** Its agent-facing MCP surface and its local adapters are feature-gated modules inside it — `mcp`, `memory`, `fs` — so one capability is one thing to find, name, and compose. A mesh node compiles in the bricks it needs and gets each one's tool namespace, schema, and authorization scope with it. No feature is on by default, so the default build is the framework-free capability.

The cost is honest and bounded: Cargo unifies features per build graph, so the workspace asserts framework-free **source** (each brick's default build resolves no adapter dependency, checked by `make isolation-check`) but does **not** claim framework-free **artifacts** — a binary composing several bricks over MCP links one framework-carrying build of each. What the crate boundary used to enforce structurally is now a path rule in the registry validator: adapter dependencies appear only under their own feature-gated module, and no feature-conditional attribute lands on a domain type.

In-process brick-to-brick calls use the consumed port trait directly and never round-trip through MCP. MCP is the door agents come through, not the wiring between rooms.

One brick is deliberately not agent-operable. `policy` decides what an agent is permitted to do, so exposing it through MCP would be a privilege-escalation seam regardless of which tools were chosen. It stays a typed Rust contract.

## Delivery order

1. Build an MCP-exposed project brick that turns a declarative blueprint into a validated Rust workspace.
2. Add an agent brick and a small local agent runtime with provider, tools, memory, knowledge, and sandbox ports.
3. Add workflow and evaluation for durable autonomous work and evidence-based acceptance.
4. Add mesh and edge adapters only after local contracts are stable.

Do not begin with a bespoke distributed-agent framework. Prove the local, MCP-driven path first; then extend the same contracts to edge and mesh deployment.
