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
brick core
  ├── MCP adapter       agent authoring and operational control
  ├── Rust SDK / traits embedded or local Rust agents
  └── mesh adapter      offline and peer-to-peer coordination
```

The core owns typed domain models, rules, validation, and stable traits. Adapters own MCP, storage, model-provider, network, and framework details. Dependencies flow inward: adapters depend on core; core never depends on adapters.

MCP is the Factory control plane, not the universal runtime API. Every brick SHOULD provide a bounded MCP surface for discovery and agent operation, but embedded applications SHOULD invoke the typed Rust API directly. Mesh communication SHOULD use a protocol selected for peer discovery, latency, offline operation, and device constraints—not an MCP round trip.

## Composition and deployment topology

MCP adapters are libraries. A thin binary composition root owns process topology: Tokio startup, transport binding, configuration, host-derived trusted context, Policy resolver construction, concrete adapter injection, and orderly shutdown. Cores and MCP libraries do not choose a process topology.

A consuming core calls another capability through the consumed capability's typed port. Local and edge composition inject direct Rust implementations. A remote client is an opt-in adapter that implements that same consumed port only after a dedicated process-boundary specification proves trusted context is independently derived at the receiver and defines authorization, request/result ceilings, idempotency, deadline/cancellation propagation, evidence, and honest recovery guarantees. MCP client calls are never hardcoded in core orchestration.

A linked edge binary is a local composition topology, not a mesh adapter. It inherits the guarantees of its selected local adapters and makes no mesh, durable recovery, cross-process cancellation, or distributed Policy propagation claim without separately specified adapters.

## Three planes

1. **Authoring plane — MCP.** Agents discover bricks and use bounded tools to create projects, configure agents, retrieve steering/skills/knowledge, operate workflows, and inspect evaluation evidence.
2. **Execution plane — native Rust.** A deployed agent uses typed ports such as `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, `Sandbox`, `ModelProvider`, and `MessageBus`. Concrete implementations remain replaceable adapters.
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
<brick>-core       domain models, rules, traits, and errors
<brick>-memory     deterministic local adapter
<brick>-<vendor>   optional storage/model/network adapter
<brick>-mcp        MCP control-plane adapter library
<brick>-server     optional thin process composition root
<brick>-mesh       optional peer-coordination adapter
mcp-transport     shared bounded MCP transport adapter, never a core
```

Not every brick needs every adapter. Start with the smallest set that proves a real capability. Avoid bespoke abstractions where the standard library or a mature framework already provides the required primitive.

The Rust-SME-approved [Canonical Brick Standard](../specs/brick-standard/requirements.md) is the mandatory scaffold contract for new or refactored portfolio entries. It defines role eligibility, crate/test layout, inward dependency direction, strict boundary DTO conversion, explicit domain validation, policy-before-effect behavior, safe bounded egress, and binary-owned process lifecycle. It is intentionally Rust-native: `serde` and `schemars` describe bounded transport/configuration DTOs; validated Rust types and core rules own domain validity. The shared transport migration is complete, but the existing Agent, Project, Workflow, and Evaluation MCP libraries retain `serve_stdio()` lifecycle ownership until their separately gated, behavior-preserving server migrations are complete; new and refactored bricks follow the standard immediately.

## Brick portfolio scaffold tracker

GitHub [Issue #9](https://github.com/bannff/Rust-Factory/issues/9) tracks this rollout. The table is the family-level source of truth for taxonomy, mandatory first scaffold, intended mature shape, and implementation state. Every **capability family** that is not already implemented receives the same status-only `<family>-core` tree before it owns behavior; adapter infrastructure, composition bases, and optional domain packs are catalogued here but do not receive fake capability crates.

```text
crates/<family>-core/
  Cargo.toml                         # [package.metadata.rust-factory]
  src/{lib.rs,model.rs,validation.rs,error.rs,port.rs,service.rs}
  tests/public_contract.rs
```

A status-only tree has `family`, `role = "core"`, and `status = "scaffolded"` in Cargo metadata; contains only compile-safe status documentation/comments; has no non-stdlib dependencies, public semantic API, or guarantee claim. When a family becomes real, its existing paths gain the typed model, validation, error, port, service, and contract-test contents—agents never need to guess where a concern belongs. The closed package statuses are `scaffolded`, `specified`, `implemented`, `migration-pending`, and `deprecated`. A deterministic metadata/layout validator enforces this registry through `make check`.

| Family | Taxonomy | Mandatory first scaffold | Mature shape when justified | Current state |
|---|---|---|---|---|
| Project authoring | Capability | Existing `project-core` migration | `project-core`, `project-fs`, `project-mcp`, `project-server` | Implemented; metadata/lifecycle migration pending |
| Workspace governance | Capability | `workspace-governance-core` status-only tree | core, Cargo/governance adapters, MCP, server | Scaffolded |
| Identity / authentication | Capability | `identity-core` status-only tree | core, trusted-host/provider adapters, MCP, server | Scaffolded; process-boundary Policy prerequisite |
| Policy / authorization | Capability | Existing `policy-core` migration | `policy-core`, local/durable/provider resolvers, MCP, server | Implemented; process-boundary semantics pending (#5) |
| Agent | Capability | Existing `agent-core` migration | core, model/tool/memory/knowledge/sandbox adapters, MCP, server | Implemented; local server #6 pending |
| Model gateway | Capability | `model-gateway-core` status-only tree | core, provider adapters, MCP, server | Scaffolded; Agent port exists; independent extraction remains pending |
| Memory | Capability | `memory-core` status-only tree | core, memory/durable/index adapters, MCP, server | Scaffolded; Agent port extraction requires one-way migration |
| Knowledge | Capability | `knowledge-core` status-only tree | core, local/index/vector/graph adapters, MCP, server | Scaffolded; Agent port extraction requires one-way migration |
| Tools / test execution | Capability | `tool-execution-core` status-only tree | core, test/tool adapters, MCP, server | Scaffolded; Agent tool port exists |
| Sandbox | Capability | `sandbox-core` status-only tree | core, deny/local/confined provider adapters, MCP, server | Scaffolded; deny adapter exists |
| Workflow | Capability | Existing `workflow-core` migration | `workflow-core`, local/durable adapters, MCP, server | Implemented; durable adapter pending |
| Evaluation | Capability | Existing `evaluation-core` migration | `evaluation-core`, local/evaluator adapters, MCP, server | Implemented; evaluator portfolio pending |
| Verification | Capability | `verification-core` status-only tree | core, live-evidence adapters, MCP, server | Scaffolded |
| Message bus / events | Capability | `message-bus-core` status-only tree | core, local/durable/broker adapters, MCP, server | Scaffolded; separate semantic spec required |
| Cache | Capability | `cache-core` status-only tree | core, local/Redis-like adapters, MCP, server | Scaffolded; separate semantic spec required |
| Graph / provenance | Capability | `graph-core` status-only tree | core, local/database adapters, MCP, server | Scaffolded; separate semantic spec required |
| Observability / audit | Capability | `observability-core` status-only tree | core, logging/tracing/metrics/audit adapters, MCP, server | Scaffolded |
| Notification | Capability | `notification-core` status-only tree | core, local/provider adapters, MCP, server | Scaffolded |
| Configuration | Adapter infrastructure | No capability crate; server config modules | bounded config-source adapters owned by composition binaries | Agent server config specified |
| Storage | Adapter infrastructure | No generic storage crate | capability-owned persistence ports and adapters | Local adapters exist; generic facade prohibited |
| HTTP / integrations | Adapter infrastructure | No capability crate | consumed-port remote/integration adapters | Deferred pending #5 |
| Browser automation | Optional capability/domain pack | `browser-core` only when an approved automation contract exists | core, bounded browser adapters, MCP, server | Optional; not scaffolded |
| MCP transport | Adapter infrastructure | Existing `mcp-transport` migration | bounded server transport adapter only | Implemented; lifecycle migration pending in consumers |
| MCP / API / worker / edge | Composition bases | No core scaffold | selected binary roots plus config/composition modules | Agent server #6 specified; others pending |
| Data / learning | Optional domain packs | Portfolio registry only until a concrete product contract exists | dataset/ML/learning capability families | Deferred |
| Security, payments, blockchain, games, UI, OpenArcade | Optional domain packs | Portfolio registry only | product-specific composition from generic seams | Excluded from generic core contracts |

A mature role is instantiated only after its own eligibility is met: `-memory` after a concrete stateful core port; `-<vendor>` to implement an existing port; `-mcp` after a bounded operational surface; `-server` after topology/configuration is specified; and `-mesh` only after a dedicated process-boundary specification. Existing packages migrate behavior-preservingly to this metadata/layout standard. Project Blueprint V1 remains a single-package generator and SHALL NOT stamp family scaffolds; a separately approved successor will do so.

## Delivery order

1. Build an MCP-exposed project brick that turns a declarative blueprint into a validated Rust workspace.
2. Add an agent brick and a small local agent runtime with provider, tools, memory, knowledge, and sandbox ports.
3. Add workflow and evaluation for durable autonomous work and evidence-based acceptance.
4. Add mesh and edge adapters only after local contracts are stable.

Do not begin with a bespoke distributed-agent framework. Prove the local, MCP-driven path first; then extend the same contracts to edge and mesh deployment.
