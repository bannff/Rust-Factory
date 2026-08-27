# Tasks

GitHub issue: #6

- [ ] Add the `agent-server` binary package as a root workspace member with only the specified exact direct dependencies; prove no `agent` or MCP-library dependency direction changes.
- [ ] Implement closed, versioned `AgentServerConfigV1` JSON decoding from one bounded `--config-json` argument; use `deny_unknown_fields`, enforce all bootstrap collection/string ceilings before adapter construction, and convert through existing Agent/Policy validation.
- [ ] Implement private launch-argument parsing and `ProcessTrustedContextSource`; require valid `--tenant-id` and `--principal-id`, use a non-wrapping synchronization-safe sequence to allocate distinct valid request/correlation IDs per `resolve`, and prohibit config/MCP identity input.
- [ ] Compose only existing deterministic local Agent adapters and configured `StaticPolicyResolver`; preserve process-local/global-definition and default-deny semantics.
- [ ] Start the existing `AgentDefinitionMcp` through `serve_stdio()` without a duplicate router or direct rmcp/mcp-transport binding.
- [ ] Add deterministic tests for strict config rejection, conversion/construction, launch identity validation and per-operation IDs, and configured-grant/default-deny behavior.
- [ ] Add a bounded stdio process smoke test for an allowed caller and a denied-identity counterpart without MCP-supplied identity or policy fields; prove malformed or oversized bootstrap configuration exits before MCP output or entry to the stdio loop.
- [ ] Run `cargo test -p agent-server -p agent -p agent-mcp -p policy-memory -p mcp-transport`, then QA, security, final architecture, final Rust SME, and `make check`.
- [ ] Update this checklist, relevant README/Blueprint wording if demonstrated by implementation evidence, and close GitHub Issue #6 only after all gates approve.

## Acceptance evidence

- Rust SME approves the binary boundary, direct dependency set, config data model, trusted-launcher context semantics, and adapter ownership before implementation.
- QA approves the strict-config, local-context, default-deny, construction, and allowed/denied stdio smoke coverage.
- Security approves identity provenance, no untrusted identity/config fallback, default deny, bounded existing MCP ingress, and no false durability or sandbox claims.
- Architecture and Rust SME approve that the final server is a thin local composition root and leaves `agent` unchanged.
- The focused server suite and `make check` pass.

Remote Policy semantics, edge composition, async Agent ports, production identity/authentication, provider adapters, sandbox execution, and durable state remain separately tracked work.
