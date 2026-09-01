//! Adapter connecting [`SandboxEventSink`] to an injected
//! [`observability::TelemetrySink`].
//!
//! `SandboxEventSink::try_emit` must remain synchronous, non-blocking, and
//! best-effort: implementations "must not perform network or disk I/O" at
//! the call site (see [`crate::SandboxEventSink`]'s doc contract). This
//! adapter honors that by checking the injected sink's
//! `TelemetryGuarantees::may_block` exactly once, at construction, and
//! refusing to ever call `emit` on a sink that may block: a `may_block`
//! sink causes every subsequent `try_emit` to return
//! [`EventSubmission::Dropped`] without touching the sink at all.
//!
//! `sandbox::CorrelationId` is forwarded only as a plain `"correlation_id"`
//! attribute string. It is never mapped onto `observability::TraceContextV1`
//! or any trace-context-shaped field: the two concepts are related but
//! distinct (see `observability::TraceContextV1`'s own doc note), and this
//! adapter does not conflate them.

use std::collections::BTreeMap;

use observability::{Clock, EventName, EventTarget, Severity, TelemetryEnvelopeV1};

use crate::{EventSubmission, SandboxEvent, SandboxEventSink, SandboxOperation, SandboxStatus};

const EVENT_TARGET: &str = "sandbox";

/// Wraps an injected `observability::TelemetrySink` and `Clock` as a
/// [`SandboxEventSink`]. See the module docs for the non-blocking and
/// non-conflation obligations this adapter honors.
pub struct TelemetryEventSink<S, C> {
    sink: S,
    clock: C,
    may_block: bool,
}
impl<S: observability::TelemetrySink, C: Clock> TelemetryEventSink<S, C> {
    /// `sink.guarantees().may_block` is read exactly once here, not on every
    /// `try_emit` call: it is documented as a static property of the sink,
    /// not a per-invocation check.
    #[must_use]
    pub fn new(sink: S, clock: C) -> Self {
        let may_block = sink.guarantees().may_block;
        Self {
            sink,
            clock,
            may_block,
        }
    }
}
impl<S: observability::TelemetrySink, C: Clock> SandboxEventSink for TelemetryEventSink<S, C> {
    fn try_emit(&self, event: SandboxEvent) -> EventSubmission {
        if self.may_block {
            return EventSubmission::Dropped;
        }
        let Ok(tenant_id) = observability::TenantId::new(event.tenant_id.as_str()) else {
            return EventSubmission::Dropped;
        };
        let Ok(timestamp) = self.clock.now() else {
            return EventSubmission::Dropped;
        };
        let Ok(name) = EventName::new(operation_name(event.operation)) else {
            return EventSubmission::Dropped;
        };
        let Ok(target) = EventTarget::new(EVENT_TARGET) else {
            return EventSubmission::Dropped;
        };
        let mut attributes = BTreeMap::new();
        attributes.insert(
            "correlation_id".to_owned(),
            event.correlation_id.as_str().to_owned(),
        );
        if let Some(sandbox_id) = &event.sandbox_id {
            attributes.insert("sandbox_id".to_owned(), sandbox_id.as_str().to_owned());
        }
        if let Some(status) = event.status {
            attributes.insert("status".to_owned(), status_name(status).to_owned());
        }
        let Ok(telemetry_event) = observability::TelemetryEventV1::new(
            name,
            target,
            Severity::Info,
            String::new(),
            attributes,
        ) else {
            return EventSubmission::Dropped;
        };
        let envelope = TelemetryEnvelopeV1 {
            tenant_id,
            timestamp,
            event: telemetry_event,
        };
        match self.sink.emit(envelope) {
            Ok(()) => EventSubmission::Accepted,
            Err(_) => EventSubmission::Dropped,
        }
    }
}

const fn operation_name(operation: SandboxOperation) -> &'static str {
    match operation {
        SandboxOperation::Start => "start",
        SandboxOperation::Execute => "execute",
        SandboxOperation::Status => "status",
        SandboxOperation::Stop => "stop",
    }
}

const fn status_name(status: SandboxStatus) -> &'static str {
    match status {
        SandboxStatus::Running => "running",
        SandboxStatus::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use observability::{ObservabilityError, TelemetryGuarantees, Timestamp};

    use super::*;
    use crate::{CorrelationId, SandboxId, TenantId};

    struct SpySink {
        guarantees: TelemetryGuarantees,
        captured: Mutex<Vec<TelemetryEnvelopeV1>>,
    }
    impl observability::TelemetrySink for SpySink {
        fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
            self.captured.lock().expect("captured").push(envelope);
            Ok(())
        }
        fn guarantees(&self) -> TelemetryGuarantees {
            self.guarantees
        }
    }

    struct FailingSink;
    impl observability::TelemetrySink for FailingSink {
        fn emit(&self, _: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
            Err(ObservabilityError::AdapterFailure)
        }
        fn guarantees(&self) -> TelemetryGuarantees {
            non_blocking_guarantees()
        }
    }

    struct FixedClock(u64);
    impl Clock for FixedClock {
        fn now(&self) -> Result<Timestamp, ObservabilityError> {
            Ok(Timestamp::from_unix_nanos(self.0))
        }
    }

    const fn non_blocking_guarantees() -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: false,
        }
    }

    const fn blocking_guarantees() -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: true,
        }
    }

    fn sandbox_event() -> SandboxEvent {
        SandboxEvent {
            operation: SandboxOperation::Execute,
            sandbox_id: Some(SandboxId::new("sbx-0123456789abcdef0123456789abcdef").expect("id")),
            status: Some(SandboxStatus::Running),
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            correlation_id: CorrelationId::new("corr-1").expect("correlation"),
        }
    }

    #[test]
    fn a_may_block_sink_is_never_called_and_every_emit_is_dropped() {
        let sink = SpySink {
            guarantees: blocking_guarantees(),
            captured: Mutex::new(Vec::new()),
        };
        let adapter = TelemetryEventSink::new(sink, FixedClock(1));
        assert_eq!(adapter.try_emit(sandbox_event()), EventSubmission::Dropped);
        assert!(adapter.sink.captured.lock().expect("captured").is_empty());
    }

    #[test]
    fn a_non_blocking_sink_accepts_and_captures_the_mapped_envelope() {
        let sink = SpySink {
            guarantees: non_blocking_guarantees(),
            captured: Mutex::new(Vec::new()),
        };
        let adapter = TelemetryEventSink::new(sink, FixedClock(7));
        assert_eq!(adapter.try_emit(sandbox_event()), EventSubmission::Accepted);
        let captured = adapter.sink.captured.lock().expect("captured");
        assert_eq!(captured.len(), 1);
        let envelope = &captured[0];
        assert_eq!(envelope.tenant_id.as_str(), "tenant-a");
        assert_eq!(envelope.timestamp, Timestamp::from_unix_nanos(7));
        assert_eq!(envelope.event.name.as_str(), "execute");
        assert_eq!(envelope.event.target.as_str(), "sandbox");
        assert_eq!(
            envelope
                .event
                .attributes
                .get("correlation_id")
                .map(String::as_str),
            Some("corr-1")
        );
        assert_eq!(
            envelope
                .event
                .attributes
                .get("sandbox_id")
                .map(String::as_str),
            Some("sbx-0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            envelope.event.attributes.get("status").map(String::as_str),
            Some("running")
        );
    }

    #[test]
    fn correlation_id_never_appears_outside_the_plain_attribute_string() {
        let sink = SpySink {
            guarantees: non_blocking_guarantees(),
            captured: Mutex::new(Vec::new()),
        };
        let adapter = TelemetryEventSink::new(sink, FixedClock(1));
        adapter.try_emit(sandbox_event());
        let captured = adapter.sink.captured.lock().expect("captured");
        let envelope = &captured[0];
        // Only one place carries the correlation id: the attribute map.
        // Nothing on TelemetryEventV1/TelemetryEnvelopeV1 resembling a trace
        // context is populated by this adapter.
        assert_eq!(envelope.event.attributes.len(), 3);
        assert!(envelope.event.attributes.contains_key("correlation_id"));
    }

    #[test]
    fn an_absent_sandbox_id_and_status_omit_their_attributes_rather_than_emitting_empty_values() {
        let sink = SpySink {
            guarantees: non_blocking_guarantees(),
            captured: Mutex::new(Vec::new()),
        };
        let adapter = TelemetryEventSink::new(sink, FixedClock(1));
        let mut event = sandbox_event();
        event.sandbox_id = None;
        event.status = None;
        adapter.try_emit(event);
        let captured = adapter.sink.captured.lock().expect("captured");
        let envelope = &captured[0];
        assert!(!envelope.event.attributes.contains_key("sandbox_id"));
        assert!(!envelope.event.attributes.contains_key("status"));
        assert!(envelope.event.attributes.contains_key("correlation_id"));
    }

    #[test]
    fn a_sink_emit_failure_is_reported_as_dropped_not_propagated() {
        let adapter = TelemetryEventSink::new(FailingSink, FixedClock(1));
        assert_eq!(adapter.try_emit(sandbox_event()), EventSubmission::Dropped);
    }

    /// Reports `false` on its first `guarantees()` call and `true` on every
    /// call after that, so a bug that re-reads `guarantees()` inside
    /// `try_emit` instead of the construction-time snapshot would flip
    /// behavior after the first call.
    struct TogglingGuaranteesSink {
        calls: Mutex<u32>,
        captured: Mutex<Vec<TelemetryEnvelopeV1>>,
    }
    impl observability::TelemetrySink for TogglingGuaranteesSink {
        fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
            self.captured.lock().expect("captured").push(envelope);
            Ok(())
        }
        fn guarantees(&self) -> TelemetryGuarantees {
            let mut calls = self.calls.lock().expect("calls");
            *calls += 1;
            if *calls == 1 {
                non_blocking_guarantees()
            } else {
                blocking_guarantees()
            }
        }
    }

    #[test]
    fn may_block_is_read_once_at_construction_not_rechecked_on_every_try_emit() {
        let sink = TogglingGuaranteesSink {
            calls: Mutex::new(0),
            captured: Mutex::new(Vec::new()),
        };
        let adapter = TelemetryEventSink::new(sink, FixedClock(1));
        // guarantees() was already called once by `new`. If try_emit read
        // guarantees() again it would see the toggled (blocking) value and
        // start dropping - but the adapter must stay pinned to the
        // construction-time snapshot (non-blocking) for every call.
        assert_eq!(adapter.try_emit(sandbox_event()), EventSubmission::Accepted);
        assert_eq!(adapter.try_emit(sandbox_event()), EventSubmission::Accepted);
        assert_eq!(adapter.try_emit(sandbox_event()), EventSubmission::Accepted);
        assert_eq!(adapter.sink.captured.lock().expect("captured").len(), 3);
    }

    #[test]
    fn try_emit_is_safe_under_concurrent_calls_from_multiple_threads() {
        use std::sync::Arc;
        use std::thread;

        let sink = SpySink {
            guarantees: non_blocking_guarantees(),
            captured: Mutex::new(Vec::new()),
        };
        let adapter = Arc::new(TelemetryEventSink::new(sink, FixedClock(1)));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let adapter = Arc::clone(&adapter);
                thread::spawn(move || {
                    let mut event = sandbox_event();
                    event.tenant_id = TenantId::new(format!("tenant-{i}")).expect("tenant");
                    for _ in 0..25 {
                        assert_eq!(adapter.try_emit(event.clone()), EventSubmission::Accepted);
                    }
                })
            })
            .collect();
        for handle in threads {
            handle.join().expect("thread panicked");
        }
        assert_eq!(adapter.sink.captured.lock().expect("captured").len(), 200);
    }

    #[test]
    fn every_sandbox_operation_maps_to_a_grammar_valid_event_name() {
        for (operation, expected) in [
            (SandboxOperation::Start, "start"),
            (SandboxOperation::Execute, "execute"),
            (SandboxOperation::Status, "status"),
            (SandboxOperation::Stop, "stop"),
        ] {
            let sink = SpySink {
                guarantees: non_blocking_guarantees(),
                captured: Mutex::new(Vec::new()),
            };
            let adapter = TelemetryEventSink::new(sink, FixedClock(1));
            let mut event = sandbox_event();
            event.operation = operation;
            assert_eq!(
                adapter.try_emit(event),
                EventSubmission::Accepted,
                "operation {expected:?} must produce a grammar-valid EventName"
            );
            let captured = adapter.sink.captured.lock().expect("captured");
            assert_eq!(captured[0].event.name.as_str(), expected);
        }
    }

    #[test]
    fn sandbox_and_observability_tenant_id_grammars_currently_agree() {
        // Regression guard for the two independently-validated TenantId
        // newtypes (sandbox::TenantId, observability::TenantId): if their
        // grammars ever diverge, this test goes red instead of silently
        // dropping events in production.
        for value in ["tenant-a", "tenant_b", "t0"] {
            assert!(TenantId::new(value).is_ok());
            assert!(observability::TenantId::new(value).is_ok());
        }
    }
}
