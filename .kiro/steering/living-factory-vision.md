---
inclusion: always
---
# Living Factory Vision

## North star

Rust Factory is a domain-agnostic, agent-operated factory for authoring, validating, deploying, and evolving reliable Rust projects and autonomous agents. An agent must be able to discover, compose, and drive bounded Factory capabilities through MCP, while deployed systems remain portable, efficient, and safe Rust programs. Polymorphic code and tools, driven by data and config - leveraging heavy on serde/schemars + pop to keep data crisp and clean.

> MCP controls (polymorphic - leveraging progressive discovery) and composes capabilities; Rust executes them locally; mesh protocols coordinate deployed peers.

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

## Three planes

1. **Authoring plane — MCP.** Agents discover bricks and use bounded tools to create projects, configure agents, retrieve steering/skills/knowledge, operate workflows, and inspect evaluation evidence.
2. **Execution plane — native Rust.** A deployed agent uses Agent-owned typed ports such as `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, and `Sandbox`, and the LLM Gateway-owned `LlmProvider` port. Concrete implementations remain replaceable adapters. A `MessageBus` port is intended but does not exist; see the capability taxonomy tracked in GitHub (issues #9 and #11).
3. **Mesh/data plane — native peer protocol.** Deployed peers discover, communicate, and replicate explicitly selected data while remaining capable of offline operation and recovery.

## Agentic control-plane ownership

- **Agents** decide, interpret, plan, and compose capabilities within explicit policy and tool scopes.
- **Bricks** execute typed, bounded operations and guarantee their contracts.
- **Workflow** owns durable lifecycle: attempt state, budgets, retries, cancellation, recovery, and terminal reasons.
- **Evaluation** independently assesses evidence and acceptance criteria.
- **Provider and framework adapters** connect models, sandboxes, storage, networks, and transports without defining the Factory core.

Agent definitions are data: identity, model policy, instructions, skills, steering, permitted tools, memory/knowledge policy, sandbox policy, and communication policy. Domain variation is data and composition, never `if domain == ...` orchestration branches.

## Storage and Cache

Storage is the authoritative versioned-object capability approved by issue #28. It owns bounded tenant- and namespace-scoped opaque object semantics; provider-specific database code is an adapter inside Storage. Capability-specific serialization, indexes, aggregate transitions, queries, and domain semantics remain owned by the capability whose data they describe rather than moving into Storage.

Cache is a separate, non-authoritative and evictable capability: losing cached data must not invalidate authoritative state. Its contract and package remain deferred and are not implied by approval of Storage.

## Mesh and CRDT safety

CRDTs are suitable for explicitly selected, eventually consistent replicated data. They do not make consequential effects safe: operations that spend resources, change authority, publish externally, or otherwise create irreversible outcomes require independently derived identity, authorization, idempotency, acknowledgement, and audit evidence, plus lease, quorum, or leader safeguards where coordination is necessary.

Delivery is local-first. Stabilize and prove the synchronous local typed contract before introducing remote, mesh, or CRDT adapters; those adapters remain opt-in boundary work behind core-owned traits and separately specified safety and recovery guarantees.

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

A brick is one crate; a module is one adapter. Exactly three roles may be separate packages because a brick cannot contain them: a deployable binary (`server`, under `projects/`), a peer-coordination adapter (`mesh`), and shared test fixtures (`test-support`).

A composition root is named for the deployable it produces, not for a capability family: it owns no capability and may host several brick MCP surfaces.

Avoid bespoke abstractions where the standard library or a mature framework already provides the required primitive.

The [Canonical Brick Standard](../specs/brick-standard/requirements.md) is the mandatory scaffold contract for new or refactored portfolio entries. It defines role eligibility, crate/test layout, inward dependency direction, strict boundary DTO conversion, explicit domain validation, policy-before-effect behavior, safe bounded egress, and binary-owned process lifecycle. The shared transport migration is complete and no library owns process lifecycle: `serve_stdio` has been removed from every brick, so transport binding belongs solely to a `projects/` binary. No such binary exists yet, so the workspace currently produces libraries only.

## Capability roadmap

The capability roadmap and taxonomy live in GitHub, not in this document. [Issue #11](https://github.com/bannff/Rust-Factory/issues/11) tracks the current taxonomy (superseding the scaffold-breadth portion of [Issue #9](https://github.com/bannff/Rust-Factory/issues/9)), and GitHub Projects tracks delivery state. This document stays focused on architecture; no table here is maintained or enforced against crate metadata.
