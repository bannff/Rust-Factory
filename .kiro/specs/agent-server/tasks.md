# Tasks

GitHub issue: #6

- [ ] Add the binary package under `projects/` with `role = "server"` and only the specified exact direct dependencies; prove no brick dependency direction changes.
- [ ] Implement closed, versioned `AgentServerConfigV1` JSON decoding from one bounded `--config-json` argument; use `deny_unknown_fields`, enforce all bootstrap collection/string ceilings before adapter construction, and convert through existing Agent/Policy validation.
- [ ] Implement private launch-argument parsing and `ProcessTrustedContextSource`; require valid `--tenant-id` and `--principal-id`, use a non-wrapping synchronization-safe sequence to allocate distinct valid request/correlation IDs per `resolve`, and prohibit config/MCP identity input.
- [ ] Compose only existing deterministic local Agent adapters and configured `StaticPolicyResolver`; preserve process-local/global-definition and default-deny semantics.
- [ ] Bind the transport in the binary: construct `mcp_transport::BoundedStdioTransport` over stdio and drive `agent::mcp::AgentDefinitionMcp` to completion, without a duplicate router and without substituting rmcp's own stdio transport, which carries no frame ceiling. `serve_stdio` no longer exists on any library, so this binary is the only thing that can restore the 64 KiB ceiling to a composed path.
- [ ] Add deterministic tests for strict config rejection, conversion/construction, launch identity validation and per-operation IDs, and configured-grant/default-deny behavior.
- [ ] Add a bounded stdio process smoke test for an allowed caller and a denied-identity counterpart without MCP-supplied identity or policy fields; prove malformed or oversized bootstrap configuration exits before MCP output or entry to the stdio loop.
- [ ] Run `cargo test -p agent --features mcp -p policy --features memory -p mcp-transport` plus the new binary's own tests, then QA, security, final architecture, final Rust SME, and `make check`.
- [ ] Update this checklist, relevant README/Blueprint wording if demonstrated by implementation evidence, and close GitHub Issue #6 only after all gates approve.

## Acceptance evidence

- Rust SME approves the binary boundary, direct dependency set, config data model, trusted-launcher context semantics, and adapter ownership before implementation.
- QA approves the strict-config, local-context, default-deny, construction, and allowed/denied stdio smoke coverage.
- Security approves identity provenance, no untrusted identity/config fallback, default deny, bounded existing MCP ingress, and no false durability or sandbox claims.
- Architecture and Rust SME approve that the final server is a thin local composition root and leaves `agent` unchanged.
- The focused server suite and `make check` pass.

Remote Policy semantics, edge composition, async Agent ports, production identity/authentication, provider adapters, sandbox execution, and durable state remain separately tracked work.
