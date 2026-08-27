---
inclusion: always
---
# Engineering Principles

1. **Polymorphic and agnostic.** Model variation through traits, generics, enums, and data—not domain-specific branches. Keep core logic independent of transports, storage, and vendors.
2. **Framework-first.** Prefer standard-library, language, and framework primitives over bespoke abstractions. Introduce custom infrastructure only when an identified gap requires it.
3. **Data-driven and test-driven.** Drive behavior from typed data and configuration where practical. Define expected behavior with focused tests, use test evidence to guide changes, and keep tests deterministic.
