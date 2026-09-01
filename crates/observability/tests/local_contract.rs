#![cfg(feature = "local")]

use std::collections::BTreeMap;
use std::sync::Arc;

use observability::local::LocalTelemetry;
use observability::{
    EventName, EventTarget, MAX_LOCAL_EVENTS_PER_TENANT, MAX_QUERY_LIMIT, ObservabilityError,
    Severity, TelemetryEnvelopeV1, TelemetryEventV1, TelemetryGuarantees, TelemetryQueryV1,
    TelemetryReader, TelemetrySink, TenantId, Timestamp,
};

fn tenant(value: &str) -> TenantId {
    TenantId::new(value).expect("tenant")
}
fn envelope(
    owner: &str,
    time: u64,
    name: &str,
    target: &str,
    severity: Severity,
) -> TelemetryEnvelopeV1 {
    TelemetryEnvelopeV1 {
        tenant_id: tenant(owner),
        timestamp: Timestamp::from_unix_nanos(time),
        event: TelemetryEventV1::new(
            EventName::new(name).expect("name"),
            EventTarget::new(target).expect("target"),
            severity,
            format!("body-{time}"),
            BTreeMap::new(),
        )
        .expect("event"),
    }
}
fn all(limit: usize) -> TelemetryQueryV1 {
    TelemetryQueryV1::new(limit).expect("query")
}

#[test]
fn constructor_capacity_and_truthful_guarantees_are_exact() {
    assert!(matches!(
        LocalTelemetry::new(0),
        Err(ObservabilityError::LimitExceeded)
    ));
    assert!(LocalTelemetry::new(MAX_LOCAL_EVENTS_PER_TENANT).is_ok());
    assert!(matches!(
        LocalTelemetry::new(MAX_LOCAL_EVENTS_PER_TENANT + 1),
        Err(ObservabilityError::LimitExceeded)
    ));
    let store = LocalTelemetry::new(1).expect("store");
    let guarantees = TelemetrySink::guarantees(&store);
    assert_eq!(
        guarantees,
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: true,
            queryable: true,
            may_block: false,
        }
    );
}

#[test]
fn ingress_and_query_are_revalidated_by_the_adapter() {
    let store = LocalTelemetry::new(2).expect("store");
    let mut invalid = envelope("tenant", 1, "event", "target", Severity::Info);
    invalid.event.body = "x".repeat(observability::MAX_BODY_BYTES + 1);
    assert_eq!(store.emit(invalid), Err(ObservabilityError::LimitExceeded));
    assert_eq!(
        store.query(
            &tenant("tenant"),
            &TelemetryQueryV1 {
                limit: 0,
                since: None,
                until: None,
                minimum_severity: None,
                event_name: None,
                target: None
            }
        ),
        Err(ObservabilityError::InvalidQuery)
    );
    assert_eq!(
        store.query(
            &tenant("tenant"),
            &TelemetryQueryV1 {
                limit: MAX_QUERY_LIMIT + 1,
                since: None,
                until: None,
                minimum_severity: None,
                event_name: None,
                target: None
            }
        ),
        Err(ObservabilityError::LimitExceeded)
    );
}

#[test]
fn tenant_isolation_newest_first_sequence_and_oldest_eviction_are_deterministic() {
    let store = LocalTelemetry::new(3).expect("store");
    store
        .emit(envelope("alpha", 30, "event", "target", Severity::Info))
        .expect("emit");
    store
        .emit(envelope("beta", 99, "event", "target", Severity::Info))
        .expect("emit");
    store
        .emit(envelope("alpha", 10, "event", "target", Severity::Info))
        .expect("emit");
    store
        .emit(envelope("alpha", 20, "event", "target", Severity::Info))
        .expect("emit");
    store
        .emit(envelope("alpha", 5, "event", "target", Severity::Info))
        .expect("emit evict");

    let alpha = store.query(&tenant("alpha"), &all(3)).expect("query");
    assert_eq!(
        alpha
            .iter()
            .map(|r| r.envelope.timestamp.as_unix_nanos())
            .collect::<Vec<_>>(),
        [5, 20, 10],
        "newest means insertion/sequence order, not caller timestamp"
    );
    assert_eq!(
        alpha.iter().map(|r| r.sequence).collect::<Vec<_>>(),
        [5, 4, 3]
    );
    assert!(
        alpha
            .iter()
            .all(|r| r.envelope.tenant_id.as_str() == "alpha")
    );
    assert_eq!(
        store
            .query(&tenant("beta"), &all(3))
            .expect("query")
            .iter()
            .map(|r| r.sequence)
            .collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn every_filter_is_conjunctive_half_open_and_limit_counts_matches() {
    let store = LocalTelemetry::new(8).expect("store");
    for fixture in [
        envelope("tenant", 10, "other", "api", Severity::Error),
        envelope("tenant", 20, "wanted", "worker", Severity::Error),
        envelope("tenant", 30, "wanted", "api", Severity::Debug),
        envelope("tenant", 40, "wanted", "api", Severity::Warn),
        envelope("tenant", 50, "wanted", "api", Severity::Error),
    ] {
        store.emit(fixture).expect("emit");
    }
    let query = TelemetryQueryV1 {
        since: Some(Timestamp::from_unix_nanos(30)),
        until: Some(Timestamp::from_unix_nanos(50)),
        minimum_severity: Some(Severity::Info),
        event_name: Some(EventName::new("wanted").expect("name")),
        target: Some(EventTarget::new("api").expect("target")),
        limit: 1,
    };
    let found = store.query(&tenant("tenant"), &query).expect("query");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].envelope.timestamp.as_unix_nanos(),
        40,
        "until excludes 50 and severity excludes 30 before the matched-result limit is applied"
    );
}

#[test]
fn concurrent_writers_preserve_all_sequences_and_tenant_partitions() {
    const THREADS: u64 = 8;
    const WRITES: u64 = 32;
    let capacity = usize::try_from(THREADS * WRITES).expect("small test capacity");
    let write_limit = usize::try_from(WRITES).expect("small test limit");
    let shared = Arc::new(LocalTelemetry::new(capacity).expect("store"));
    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let shared = Arc::clone(&shared);
            scope.spawn(move || {
                for index in 0..WRITES {
                    shared
                        .emit(envelope(
                            &format!("t-{thread}"),
                            index,
                            "event",
                            "target",
                            Severity::Info,
                        ))
                        .expect("emit");
                }
            });
        }
    });
    let mut sequences = Vec::new();
    for thread in 0..THREADS {
        let found = shared
            .query(&tenant(&format!("t-{thread}")), &all(write_limit))
            .expect("query");
        assert_eq!(found.len(), write_limit);
        sequences.extend(found.into_iter().map(|record| record.sequence));
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=THREADS * WRITES).collect::<Vec<_>>());
}

#[test]
fn aggregate_event_and_tenant_cardinality_is_bounded() {
    let store = LocalTelemetry::new(1).expect("store");
    for index in 0..observability::MAX_LOCAL_EVENTS_TOTAL {
        store
            .emit(envelope(
                &format!("tenant-{index}"),
                u64::try_from(index).expect("bounded index"),
                "event",
                "target",
                Severity::Info,
            ))
            .expect("fill bounded store");
    }

    store
        .emit(envelope(
            "tenant-0",
            9_999,
            "event",
            "target",
            Severity::Info,
        ))
        .expect("a full tenant may evict its oldest event without growing the store");
    assert_eq!(
        store.emit(envelope(
            "one-more-tenant",
            10_000,
            "event",
            "target",
            Severity::Info,
        )),
        Err(ObservabilityError::LimitExceeded)
    );
    assert_eq!(
        store
            .query(&tenant("tenant-0"), &all(1))
            .expect("query retained tenant")[0]
            .envelope
            .timestamp
            .as_unix_nanos(),
        9_999
    );
}
