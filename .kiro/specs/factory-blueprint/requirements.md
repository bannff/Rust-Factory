# Rust Factory Blueprint

## Purpose

Define the capability portfolio and staged construction plan for a domain-agnostic, MCP-operated Rust Factory. Python Factory is a capability reference; Rust Factory keeps its proven ownership boundaries while using idiomatic Cargo crates, traits, and adapters.

## Principles

- Bricks use domain nouns: Project, Agent, Workflow, Evaluation, Memory, Knowledge, Sandbox, Tool, and Policy.
- Every core is typed, transport-independent, versioned, bounded, and framework-free.
- Adapters depend inward; MCP is control plane, native Rust is execution plane, and mesh is deferred data-plane work.
- Agent decides; brick executes; Workflow owns durable lifecycle; Evaluation assesses immutable evidence.
- All external effects require trusted context, authorization, limits, idempotency, and evidence.
- Framework choice is project-level declarative adapter composition: cores define stable ports, named adapter or experiment crates contain frameworks, and Cargo plus constructor injection performs actual composition.
- Experiments emit immutable evidence; an Evaluation PASS never auto-promotes an adapter, project, or SDK change.

See `adapter-portfolio.md` for the bounded adapter selection, experiment, and deferred promotion doctrine.

## Acceptance

The portfolio, Cargo tree, framework policy, testing matrix, migration map, and roadmap are explicit; no brick is scaffolded until its own spec passes the Rust SME gate.