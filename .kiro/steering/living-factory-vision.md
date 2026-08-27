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
<brick>-mcp        MCP control-plane adapter
<brick>-mesh       optional peer-coordination adapter
```

Not every brick needs every adapter. Start with the smallest set that proves a real capability. Avoid bespoke abstractions where the standard library or a mature framework already provides the required primitive.

## Delivery order

1. Build an MCP-exposed project brick that turns a declarative blueprint into a validated Rust workspace.
2. Add an agent brick and a small local agent runtime with provider, tools, memory, knowledge, and sandbox ports.
3. Add workflow and evaluation for durable autonomous work and evidence-based acceptance.
4. Add mesh and edge adapters only after local contracts are stable.

Do not begin with a bespoke distributed-agent framework. Prove the local, MCP-driven path first; then extend the same contracts to edge and mesh deployment.
