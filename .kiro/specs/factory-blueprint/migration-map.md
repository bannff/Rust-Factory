# Python Factory Migration Map

| Python capability | Rust disposition |
|---|---|
| Polylith components/bases | Cargo domain crates plus core/adapter suffixes |
| Pydantic models | typed Rust models + explicit validation; serde/schemars at boundaries |
| Protocol ports | Rust traits owned by core crates |
| MCP bricks | bounded rmcp adapters |
| Agent registry/runtime | Agent brick, already implemented locally |
| Workflow lifecycle | Workflow brick, already implemented locally with explicit local-only limits |
| Evals evidence | Evaluation brick, implemented pending final gates |
| Dynamic imports/service locators | do not port; use explicit injection/catalogs and Cargo-selected named adapter crates |
| Framework menus (UI/provider/orchestration/etc.) | bounded declarative adapter selections validated during project planning; framework code stays in named adapter/experiment crates |
| Experiment harnesses and framework comparison | evidence-producing experiments around existing Workflow/Evaluation contracts; promotion is a separately specified projection |
| Celery/Dagster/backend switches | defer; specify durable adapters independently |
| Companion memory | Memory brick implemented with local and agentic adapters, settings, and MCP; no durable adapter |
| Companion knowledge | Knowledge brick and Agent one-way migration implemented under issue #37; QA/security and final Rust SME/meta-architecture reviews approved with no Blocker or Required findings; status promotion to `implemented`, focused matrix, and `make check` complete; only issue evidence, merge, and delivery pending |
| UI/dashboard | defer; no UI framework in core |
| Graphs/swarms | defer until local agent/workflow contracts demonstrate need |
