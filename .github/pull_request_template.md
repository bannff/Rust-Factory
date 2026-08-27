## Summary

Describe the behavior change and its intended outcome.

## Validation

- [ ] `make check` passes.
- [ ] Focused tests cover the changed behavior, or this change needs no focused test.
- [ ] Core-to-adapter dependency direction remains intact; core crates do not depend on adapters.
- [ ] Any added or changed dependency uses an exact Cargo version pin.

## Safety evidence

- [ ] MCP scope, authorization, input/output bounds, and tool exposure were reviewed where relevant.
- [ ] Filesystem paths and side effects remain confined and validated where relevant.
- [ ] Async/concurrency cancellation, resource bounds, and state transitions were reviewed where relevant.
