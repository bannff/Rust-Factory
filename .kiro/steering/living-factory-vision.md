---
inclusion: always
---
# Living Factory Vision

## North star

Rust Factory is a domain-agnostic, agent-operated factory for authoring, validating, deploying, and evolving reliable Rust projects and autonomous agents. Agents discover, compose, and drive bounded Factory capabilities through MCP while deployed systems remain portable, efficient, and safe Rust programs.

> MCP controls and composes capabilities; Rust executes them locally; mesh protocols coordinate deployed peers.

This document is a living architectural north star. Update it as validated implementation evidence changes the design.

## One capability, mandatory agent surface

Every mature capability brick has a transport-independent Rust core, an always-available typed Rust API, and a mandatory feature-gated MCP handler:

```text
one brick crate
  ├── typed Rust API    embedded and local callers                 (always)
  ├── mcp handler       bounded agent control-plane contribution   (feature)
  └── mesh adapter      offline and peer coordination              (separate crate)
```

The core owns typed domain models, rules, validation, and stable traits. Adapters own MCP, storage, provider, network, and framework details. Dependencies flow inward.

Each MCP handler is transport-agnostic and self-describing. It SHALL expose generated, bounded `<namespace>_capabilities` and `<namespace>_schema` tools even when the safe operational surface is introspection-only. Sensitive bricks such as Policy, Auth, Storage, Knowledge, and LLM Gateway SHALL NOT expose privilege or raw-bypass operations: no caller-supplied authorization decisions, grant mutation, token minting, raw object CRUD, unrestricted corpus access, or unrestricted provider invocation.

MCP is the Factory control plane, not the in-process runtime API. Embedded callers and project orchestration use typed Rust APIs and consumed ports; they never round-trip through MCP. Mesh communication uses a protocol selected for peer discovery, latency, offline operation, and device constraints—not MCP.

## Composition and deployment topology

Every deployable project has exactly one binary target, process, and unified MCP server/router over `BoundedStdioTransport`. The composition root statically combines selected brick `HandlerContribution`s and separately typed `factory_*` project meta-tools behind one validation, budget, discovery, and dispatch contract. There are no per-brick server processes or dynamic handler discovery.

The binary owns runtime startup, transport binding, aggregate ingress/egress and discovery ceilings, duplicate-name rejection, configuration, host-derived trusted context, Policy construction, concrete adapter injection, admission/cancellation, and orderly shutdown. Brick handlers own no process topology.

Namespaces SHALL match `[a-z][a-z0-9_]{0,63}`; crate hyphens normalize to underscores. Brick tools use `<namespace>_<operation>`, while project meta-tools exclusively use `factory_<operation>`. Startup fails closed on invalid or duplicate namespaces/tool names and aggregate schema/discovery budget violations.

Before every write, `BoundedStdioTransport` SHALL serialize and measure the complete `TxJsonRpcMessage`, including the caller-controlled JSON-RPC request ID. It SHALL fail closed without a partial write when the envelope exceeds 64 KiB. Smaller handler-result limits provide headroom only; they do not prove the wire bound.

Every non-introspection brick tool and project meta-tool SHALL receive host-derived trusted context and exact Policy authorization immediately before any effect or tenant-scoped read. Caller input never establishes identity, grants, trusted context, ceilings, or host paths. Introspection remains bounded and projects no grants, tokens, paths, corpus/provider data, or backend secrets.

A remote client is an opt-in adapter only after a process-boundary specification proves independently derived receiver context, authorization, ceilings, idempotency, deadline/cancellation propagation, evidence, and honest recovery guarantees.

## Three planes

1. **Authoring plane — MCP.** Agents discover bricks and use bounded tools to create projects, configure agents, retrieve steering/skills/knowledge, operate workflows, and inspect evaluation evidence.
2. **Execution plane — native Rust.** Deployed code composes direct typed ports. Agent retains planning, policy, capability preflight, event projection, and output accounting; provider and knowledge capabilities remain replaceable injected adapters.
3. **Mesh/data plane — native peer protocol.** Deployed peers discover, communicate, and replicate explicitly selected data while remaining capable of offline operation and recovery.

## Agentic control-plane ownership

- **Agents** decide, interpret, plan, and compose capabilities within explicit policy and tool scopes.
- **Bricks** execute typed, bounded operations and guarantee their contracts.
- **Workflow** owns durable lifecycle: attempt state, budgets, retries, cancellation, recovery, and terminal reasons.
- **Evaluation** independently assesses evidence and acceptance criteria.
- **Provider and framework adapters** connect models, sandboxes, storage, networks, and transports without defining the Factory core.

Agent definitions are data: identity, model policy, instructions, skills, steering, permitted tools, memory/knowledge policy, sandbox policy, and communication policy. Domain variation is data and composition, never `if domain == ...` orchestration branches.

## Storage and Cache

Storage owns bounded tenant- and namespace-scoped opaque versioned-object semantics; provider-specific database code is an adapter inside Storage. Capability-specific serialization, indexes, aggregate transitions, queries, and domain semantics remain with the owning capability. Its mandatory MCP migration may begin introspection-only and SHALL NOT expose raw object CRUD.

Cache is separate, non-authoritative, and evictable: losing cached data must not invalidate authoritative state. Its contract and package remain deferred.

## Mesh and CRDT safety

CRDTs are suitable for explicitly selected, eventually consistent replicated data. Consequential effects still require independently derived identity, authorization, idempotency, acknowledgement, audit evidence, and coordination safeguards where necessary.

Delivery is local-first. Stabilize and prove synchronous local typed contracts before remote, mesh, or CRDT adapters; those adapters remain opt-in boundary work behind core-owned traits and separately specified safety and recovery guarantees.

## Brick anatomy and migration

```text
crates/<brick>/src/{lib,model,validation,error,port,service}.rs
  + mandatory feature-gated mcp handler and optional local/fs/vendor/settings adapters
projects/<name>/
  exactly one binary composition root and unified MCP server
```

A brick is one crate. Every mature `role = "brick"` package SHALL own the feature-gated bounded MCP handler contract; feature selection controls whether a project compiles it, not whether the brick defines it. Separate packages are limited to deployable `server`, peer `mesh`, and shared `test-support` roles. Shared `mcp-transport` infrastructure owns no capability. No library owns stdio serving or process lifecycle.

Existing mature bricks that lack `HandlerContribution` conformance or generated capabilities/schema tools are migration debt, not exceptions. A temporary explicit allowlist may track them. Removal requires compiled and runtime conformance proving the shared `HandlerContribution` contract, `<namespace>_capabilities` and `<namespace>_schema`, exact namespace/tool ownership, and Makefile feature coverage. Regex-only source evidence is insufficient. Remove entries one brick at a time and delete the allowlist when migration completes; do not claim current portfolio-wide compliance before that evidence exists.

The [Canonical Brick Standard](../specs/brick-standard/requirements.md) is the mandatory scaffold contract. The capability roadmap and taxonomy live in GitHub issue [#11](https://github.com/bannff/Rust-Factory/issues/11) and GitHub Projects.
