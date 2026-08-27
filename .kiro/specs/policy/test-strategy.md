# Policy Test Strategy

Narrow command: `cargo test -p policy -p policy-memory` (plus `-p policy-mcp` only if created).

Required: ID/context validation; default deny; tenant/principal separation; closed capability validation; duplicate grant canonicalization; decision digest golden vectors; grant intersection cannot elevate Agent scope; resolver failure deny; safe deny/not-found projections.

Conditional: proptest for grant canonicalization/intersection; loom only if a concurrent or persistent resolver appears; fuzz only for a future token/claims parser. N/A V1: generated fixture, filesystem, sandbox execution, provider, network, and mesh tests.