# Policy Framework Policy

V1 uses only the Rust standard library: structs/enums/newtypes, BTreeMap/BTreeSet, explicit validation, typed errors, and SHA-256 only if a stable decision digest needs evidence binding. No auth framework belongs in policy.

Future identity provider or Cedar-style evaluator integrations are adapter-only and require a separate spec, Rust SME approval, exact pins, trusted-context threat model, revocation semantics, and compatibility plan. `rmcp` is used only by policy-mcp if caller-relative discovery is proven necessary.