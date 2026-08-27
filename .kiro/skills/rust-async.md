---
name: rust-async
description: Async and concurrency guidance for Rust Factory agents.
inclusion: auto
---
# Rust Async

- Start synchronous. Introduce async only for real concurrent I/O, streaming, or latency needs.
- When async is required, use one runtime consistently and keep runtime-specific types in adapters.
- Make cancellation, timeout, retry, backpressure, and ownership explicit at every asynchronous boundary.
- Never hold locks, blocking I/O, or long CPU work across `.await` points.
- Prefer message passing or immutable data to shared mutable state; require a reason before using `Arc<Mutex<_>>`.
