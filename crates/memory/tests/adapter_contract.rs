//! The port conformance suite.
//!
//! Every clause of the [`memory::MemoryStore`] contract is checked here, and the
//! suite is generic over the implementation so each adapter runs the identical
//! assertions. An adapter-specific test would let two backends drift into
//! different behaviour behind the same port, which is the one failure this brick
//! cannot tolerate.
//!
//! Adding an adapter means adding a `#[test]` that calls [`run_conformance`]. If
//! it passes, the adapter is substitutable; nothing else is required of it.
//!
//! Gated on `local` because the reference adapter is the baseline every other
//! adapter is compared against.

#![cfg(feature = "local")]

use std::collections::BTreeMap;

use memory::model::Provenance;
use memory::{
    Clock, MemoryKind, MemoryQuery, MemoryRecord, MemoryStore, Namespace, RecordKey, RunId,
    TenantId, Timestamp, WriteOutcome,
};

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).expect("valid tenant")
}

fn namespace(name: &str) -> Namespace {
    Namespace::new(name).expect("valid namespace")
}

fn key(name: &str) -> RecordKey {
    RecordKey::new(name).expect("valid key")
}

/// Builds a validated record, so a malformed fixture fails the fixture rather
/// than the behaviour under test.
fn record(
    tenant_name: &str,
    namespace_name: &str,
    key_name: &str,
    kind: MemoryKind,
    content: &str,
    tags: &[&str],
    recorded_at_micros: Option<u64>,
) -> MemoryRecord {
    MemoryRecord {
        tenant_id: tenant(tenant_name),
        namespace: namespace(namespace_name),
        key: key(key_name),
        kind,
        content: content.to_owned(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        metadata: BTreeMap::new(),
        provenance: recorded_at_micros.map(|micros| Provenance {
            run_id: RunId::new("run-1").expect("valid run id"),
            recorded_at: Timestamp::from_micros(micros),
        }),
    }
    .validated()
    .expect("fixture is valid")
}

/// Runs every contract clause against one implementation.
pub fn run_conformance<S: MemoryStore>(store: &S) {
    put_then_get_round_trips(store);
    repeated_put_replaces_rather_than_duplicates(store);
    another_tenant_cannot_observe_a_record(store);
    a_namespace_does_not_leak_into_another(store);
    every_query_filter_is_honoured(store);
    a_query_never_exceeds_its_limit(store);
    delete_reports_whether_it_removed_anything(store);
    guarantees_are_stated(store);
}

/// Clause: a written record reads back with every field intact.
fn put_then_get_round_trips<S: MemoryStore>(store: &S) {
    let written = record(
        "acme",
        "notes",
        "round-trip",
        MemoryKind::Factual,
        "the build is green",
        &["ci"],
        Some(10),
    );
    assert_eq!(
        store.put(written.clone()).expect("put succeeds"),
        WriteOutcome::Created,
        "a new key must report Created"
    );
    let found = store
        .get(&tenant("acme"), &namespace("notes"), &key("round-trip"))
        .expect("get succeeds")
        .expect("record is present");
    assert_eq!(found, written, "a round trip must not alter the record");
}

/// Clause 3: writing an existing key replaces, and never duplicates.
fn repeated_put_replaces_rather_than_duplicates<S: MemoryStore>(store: &S) {
    let first = record(
        "acme",
        "replace",
        "same-key",
        MemoryKind::Factual,
        "first value",
        &[],
        Some(20),
    );
    let second = record(
        "acme",
        "replace",
        "same-key",
        MemoryKind::Factual,
        "second value",
        &[],
        Some(21),
    );
    assert_eq!(store.put(first).expect("first put"), WriteOutcome::Created);
    assert_eq!(
        store.put(second.clone()).expect("second put"),
        WriteOutcome::Replaced,
        "an existing key must report Replaced"
    );
    let found = store
        .get(&tenant("acme"), &namespace("replace"), &key("same-key"))
        .expect("get succeeds")
        .expect("record is present");
    assert_eq!(found.content, "second value", "replace must win");

    let all = store
        .query(
            &tenant("acme"),
            &namespace("replace"),
            &MemoryQuery::all(64).expect("valid query"),
        )
        .expect("query succeeds");
    assert_eq!(all.len(), 1, "replace must not leave the old record behind");
}

/// Clause 1: no operation may observe another tenant's record, and the refusal is
/// indistinguishable from absence.
fn another_tenant_cannot_observe_a_record<S: MemoryStore>(store: &S) {
    let owned = record(
        "tenant-a",
        "shared",
        "secret",
        MemoryKind::Factual,
        "only for tenant a",
        &[],
        Some(30),
    );
    store.put(owned).expect("put succeeds");

    assert!(
        store
            .get(&tenant("tenant-b"), &namespace("shared"), &key("secret"))
            .expect("get succeeds")
            .is_none(),
        "a foreign tenant must see absence, not the record"
    );
    assert!(
        store
            .query(
                &tenant("tenant-b"),
                &namespace("shared"),
                &MemoryQuery::all(64).expect("valid query")
            )
            .expect("query succeeds")
            .is_empty(),
        "a foreign tenant's query must not return the record"
    );
    assert!(
        !store
            .delete(&tenant("tenant-b"), &namespace("shared"), &key("secret"))
            .expect("delete succeeds"),
        "a foreign tenant must not be able to delete the record"
    );
    assert!(
        store
            .get(&tenant("tenant-a"), &namespace("shared"), &key("secret"))
            .expect("get succeeds")
            .is_some(),
        "the owner's record must survive a foreign delete attempt"
    );
}

/// Clause 2: the same key in two namespaces is two records.
fn a_namespace_does_not_leak_into_another<S: MemoryStore>(store: &S) {
    store
        .put(record(
            "acme",
            "space-one",
            "shared-key",
            MemoryKind::Factual,
            "in one",
            &[],
            Some(40),
        ))
        .expect("put succeeds");
    store
        .put(record(
            "acme",
            "space-two",
            "shared-key",
            MemoryKind::Factual,
            "in two",
            &[],
            Some(41),
        ))
        .expect("put succeeds");

    let one = store
        .get(&tenant("acme"), &namespace("space-one"), &key("shared-key"))
        .expect("get succeeds")
        .expect("present");
    let two = store
        .get(&tenant("acme"), &namespace("space-two"), &key("shared-key"))
        .expect("get succeeds")
        .expect("present");
    assert_eq!(one.content, "in one");
    assert_eq!(
        two.content, "in two",
        "namespaces must not share a key space"
    );
}

/// Clause 4: every filter is honoured, including by an adapter that cannot push
/// it down to storage.
fn every_query_filter_is_honoured<S: MemoryStore>(store: &S) {
    let space = "filters";
    store
        .put(record(
            "acme",
            space,
            "fact-early",
            MemoryKind::Factual,
            "alpha content",
            &["red"],
            Some(100),
        ))
        .expect("put");
    store
        .put(record(
            "acme",
            space,
            "pref-late",
            MemoryKind::Preference,
            "beta content",
            &["red", "blue"],
            Some(200),
        ))
        .expect("put");
    store
        .put(record(
            "acme",
            space,
            "proc-later",
            MemoryKind::Procedural,
            "gamma content",
            &[],
            Some(300),
        ))
        .expect("put");

    let query_for = |mutate: fn(&mut MemoryQuery)| {
        let mut query = MemoryQuery::all(64).expect("valid query");
        mutate(&mut query);
        store
            .query(&tenant("acme"), &namespace(space), &query)
            .expect("query succeeds")
            .into_iter()
            .map(|record| record.key.as_str().to_owned())
            .collect::<Vec<_>>()
    };

    let by_kind = query_for(|query| query.kinds = vec![MemoryKind::Preference]);
    assert_eq!(by_kind, vec!["pref-late"], "kind filter must apply");

    // Two tags means every tag must be present, not any.
    let by_tags = query_for(|query| query.tags = vec!["red".to_owned(), "blue".to_owned()]);
    assert_eq!(by_tags, vec!["pref-late"], "tags must be conjunctive");

    let by_term = query_for(|query| query.term = Some("gamma".to_owned()));
    assert_eq!(by_term, vec!["proc-later"], "term filter must apply");

    // `since` is inclusive and `until` exclusive, so this selects exactly the
    // record at 200.
    let by_window = query_for(|query| {
        query.since = Some(Timestamp::from_micros(200));
        query.until = Some(Timestamp::from_micros(300));
    });
    assert_eq!(
        by_window,
        vec!["pref-late"],
        "since must be inclusive and until exclusive"
    );

    let no_match = query_for(|query| query.term = Some("absent-term".to_owned()));
    assert!(
        no_match.is_empty(),
        "a term matching nothing returns nothing"
    );
}

/// Clause 5: at most `limit` records come back.
fn a_query_never_exceeds_its_limit<S: MemoryStore>(store: &S) {
    let space = "limits";
    for index in 0..5_u32 {
        store
            .put(record(
                "acme",
                space,
                &format!("key-{index}"),
                MemoryKind::Factual,
                "content",
                &[],
                Some(u64::from(index)),
            ))
            .expect("put");
    }
    let found = store
        .query(
            &tenant("acme"),
            &namespace(space),
            &MemoryQuery::all(2).expect("valid query"),
        )
        .expect("query succeeds");
    assert_eq!(found.len(), 2, "limit must bound the result");
}

/// Clause: delete reports whether a record existed, and is idempotent.
fn delete_reports_whether_it_removed_anything<S: MemoryStore>(store: &S) {
    store
        .put(record(
            "acme",
            "deletes",
            "doomed",
            MemoryKind::Factual,
            "content",
            &[],
            Some(50),
        ))
        .expect("put");
    assert!(
        store
            .delete(&tenant("acme"), &namespace("deletes"), &key("doomed"))
            .expect("delete succeeds"),
        "deleting an existing record reports true"
    );
    assert!(
        !store
            .delete(&tenant("acme"), &namespace("deletes"), &key("doomed"))
            .expect("delete succeeds"),
        "deleting again reports false rather than failing"
    );
    assert!(
        store
            .get(&tenant("acme"), &namespace("deletes"), &key("doomed"))
            .expect("get succeeds")
            .is_none(),
        "a deleted record must be gone"
    );
    assert!(
        !store
            .delete(
                &tenant("acme"),
                &namespace("deletes"),
                &key("never-existed")
            )
            .expect("delete succeeds"),
        "deleting an absent key is not an error"
    );
}

/// Clause 6: an in-process adapter must not claim durability it does not have.
fn guarantees_are_stated<S: MemoryStore>(store: &S) {
    let guarantees = store.guarantees();
    assert!(
        !guarantees.durable_across_restart,
        "an in-process store must not claim to survive a restart"
    );
    assert!(
        !guarantees.visible_across_processes,
        "an in-process store must not claim cross-process visibility"
    );
}

#[test]
fn in_process_store_is_conformant() {
    run_conformance(&memory::local::InProcessStore::new());
}

#[cfg(feature = "agentic")]
#[test]
fn agentic_memory_store_is_conformant() {
    run_conformance(&memory::agentic::AgenticMemoryInProcessStore::new());
}

/// The two adapters must agree, not merely each pass in isolation.
///
/// A clause the suite fails to check could still be implemented differently by
/// each backend, so this compares observable behaviour directly.
#[cfg(feature = "agentic")]
#[test]
fn both_adapters_agree_on_observable_behaviour() {
    let local = memory::local::InProcessStore::new();
    let agentic = memory::agentic::AgenticMemoryInProcessStore::new();

    let fixtures = [
        record(
            "acme",
            "agree",
            "one",
            MemoryKind::Factual,
            "alpha",
            &["x"],
            Some(1),
        ),
        record(
            "acme",
            "agree",
            "two",
            MemoryKind::Episodic,
            "beta",
            &["x", "y"],
            Some(2),
        ),
    ];
    for fixture in &fixtures {
        assert_eq!(
            local.put(fixture.clone()).expect("local put"),
            agentic.put(fixture.clone()).expect("agentic put"),
            "the two adapters must report the same write outcome"
        );
    }

    let mut query = MemoryQuery::all(64).expect("valid query");
    query.tags = vec!["x".to_owned()];
    let mut from_local = local
        .query(&tenant("acme"), &namespace("agree"), &query)
        .expect("local query");
    let mut from_agentic = agentic
        .query(&tenant("acme"), &namespace("agree"), &query)
        .expect("agentic query");
    from_local.sort_by(|left, right| left.key.cmp(&right.key));
    from_agentic.sort_by(|left, right| left.key.cmp(&right.key));
    assert_eq!(
        from_local, from_agentic,
        "the two adapters must return identical records for identical input"
    );
}

/// A clock is injected, so provenance is reproducible rather than wall-clock
/// dependent.
#[test]
fn a_fixed_clock_makes_provenance_deterministic() {
    let clock = memory::local::FixedClock::new(Timestamp::from_micros(4_242));
    assert_eq!(clock.now().as_micros(), 4_242);
    assert_eq!(clock.now(), clock.now(), "a fixed clock must not advance");
}

// ---------------------------------------------------------------------------
// Clause 6 and 7: validation at ingress, and no partial effect on failure.
//
// These were added after a review found that the agentic adapter deferred to its
// backend's content ceiling and mutated its index before its graph. The result
// was that an oversized write destroyed the previous record and left the key
// permanently unwritable, while the equivalent write to the local adapter simply
// succeeded. Both behaviours are now contract clauses.
// ---------------------------------------------------------------------------

/// Clause 6: an adapter rejects what the core rejects, on its own.
fn ingress_validation_is_the_adapters_own_job<S: MemoryStore>(store: &S) {
    let oversized = MemoryRecord {
        tenant_id: tenant("acme"),
        namespace: namespace("ingress"),
        key: key("too-big"),
        kind: MemoryKind::Factual,
        // Above the core ceiling. Deliberately built without `validated()`, which
        // is the whole point: a caller holding the port directly bypasses the
        // service, so the adapter is the last line of defence.
        content: "a".repeat(memory::MAX_CONTENT_BYTES + 1),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        provenance: None,
    };
    assert_eq!(
        store.put(oversized),
        Err(memory::MemoryError::LimitExceeded),
        "an adapter must apply the core's ceiling rather than its backend's"
    );

    let empty = MemoryRecord {
        tenant_id: tenant("acme"),
        namespace: namespace("ingress"),
        key: key("empty"),
        kind: MemoryKind::Factual,
        content: String::new(),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        provenance: None,
    };
    assert_eq!(
        store.put(empty),
        Err(memory::MemoryError::InvalidRecord),
        "an adapter must reject meaningless content"
    );
}

/// Clause 7: a failed write leaves the previous record intact and the key usable.
fn a_failed_write_preserves_the_previous_record<S: MemoryStore>(store: &S) {
    let space = "atomic";
    store
        .put(record(
            "acme",
            space,
            "survivor",
            MemoryKind::Factual,
            "original content",
            &[],
            Some(1),
        ))
        .expect("the first write succeeds");

    let doomed = MemoryRecord {
        tenant_id: tenant("acme"),
        namespace: namespace(space),
        key: key("survivor"),
        kind: MemoryKind::Factual,
        content: "a".repeat(memory::MAX_CONTENT_BYTES + 1),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        provenance: None,
    };
    assert!(store.put(doomed).is_err(), "the oversized write must fail");

    let found = store
        .get(&tenant("acme"), &namespace(space), &key("survivor"))
        .expect("get succeeds")
        .expect("the original record must survive a failed write");
    assert_eq!(
        found.content, "original content",
        "a failed write must not have applied any part of itself"
    );

    // The key must still be writable: a half-applied failure used to leave the
    // index pointing at a node that no longer existed.
    assert_eq!(
        store
            .put(record(
                "acme",
                space,
                "survivor",
                MemoryKind::Factual,
                "recovered content",
                &[],
                Some(2),
            ))
            .expect("the key is still writable"),
        WriteOutcome::Replaced,
        "a failed write must not poison the key"
    );
    assert!(
        store
            .delete(&tenant("acme"), &namespace(space), &key("survivor"))
            .expect("delete still works"),
        "a failed write must not poison deletion either"
    );
}

/// Clause 5, precisely: the limit counts *matching* records, not records examined.
///
/// An adapter that checks the limit before applying its filter returns too few
/// records, which a caller cannot distinguish from there being no more data.
fn a_limit_counts_matches_rather_than_records_examined<S: MemoryStore>(store: &S) {
    let space = "interleaved";
    // Alternating kinds, so a kind filter rejects every other record.
    for index in 0..10_u32 {
        let kind = if index % 2 == 0 {
            MemoryKind::Episodic
        } else {
            MemoryKind::Factual
        };
        store
            .put(record(
                "acme",
                space,
                &format!("k-{index}"),
                kind,
                "content",
                &[],
                Some(u64::from(index)),
            ))
            .expect("put");
    }

    let mut query = MemoryQuery::all(3).expect("valid query");
    query.kinds = vec![MemoryKind::Factual];
    let found = store
        .query(&tenant("acme"), &namespace(space), &query)
        .expect("query succeeds")
        .into_iter()
        .map(|record| record.key.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        vec!["k-1", "k-3", "k-5"],
        "the limit must yield three matches, in key order, skipping non-matches"
    );
}

/// Metadata must survive a round trip, including through an adapter whose backend
/// record type has no metadata field at all.
fn metadata_and_tags_round_trip<S: MemoryStore>(store: &S) {
    let written = MemoryRecord {
        tenant_id: tenant("acme"),
        namespace: namespace("sidecar"),
        key: key("rich"),
        kind: MemoryKind::Procedural,
        content: "content".to_owned(),
        tags: vec!["alpha".to_owned(), "beta".to_owned()],
        metadata: BTreeMap::from([
            ("source".to_owned(), "review".to_owned()),
            ("confidence".to_owned(), "high".to_owned()),
        ]),
        provenance: Some(Provenance {
            run_id: RunId::new("run-7").expect("valid"),
            recorded_at: Timestamp::from_micros(99),
        }),
    }
    .validated()
    .expect("fixture is valid");
    store.put(written.clone()).expect("put");

    let found = store
        .get(&tenant("acme"), &namespace("sidecar"), &key("rich"))
        .expect("get")
        .expect("present");
    assert_eq!(
        found, written,
        "tags, metadata, and provenance must survive a backend that cannot represent them"
    );

    // And they must survive a replace, which rebuilds the sidecar.
    let mut replaced = written.clone();
    "new content".clone_into(&mut replaced.content);
    store.put(replaced.clone()).expect("replace");
    let after = store
        .get(&tenant("acme"), &namespace("sidecar"), &key("rich"))
        .expect("get")
        .expect("present");
    assert_eq!(
        after, replaced,
        "a replace must rebuild the sidecar correctly"
    );
}

/// Every kind must survive a round trip, including through an adapter that maps
/// onto a different and larger vocabulary.
fn every_kind_round_trips<S: MemoryStore>(store: &S) {
    for kind in MemoryKind::all() {
        let name = format!("kind-{}", kind.as_str());
        store
            .put(record(
                "acme",
                "kinds",
                &name,
                kind,
                "content",
                &[],
                Some(1),
            ))
            .expect("put");
        let found = store
            .get(&tenant("acme"), &namespace("kinds"), &key(&name))
            .expect("get")
            .expect("present");
        assert_eq!(
            found.kind,
            kind,
            "{} must not change through a round trip",
            kind.as_str()
        );
    }
}

/// A tuple key must not let two distinct partitions alias.
///
/// The grammar permits `.` in an identifier, so a joined-string key would make
/// ("a", "b.c") and ("a.b", "c") collide. Both adapters use a tuple today; this
/// pins that down so a future change to a concatenated key cannot pass silently.
fn distinct_tenant_and_namespace_pairs_never_alias<S: MemoryStore>(store: &S) {
    store
        .put(
            MemoryRecord {
                tenant_id: TenantId::new("a").expect("valid"),
                namespace: Namespace::new("b.c").expect("valid"),
                key: key("same"),
                kind: MemoryKind::Factual,
                content: "first pair".to_owned(),
                tags: Vec::new(),
                metadata: BTreeMap::new(),
                provenance: None,
            }
            .validated()
            .expect("valid"),
        )
        .expect("put");
    store
        .put(
            MemoryRecord {
                tenant_id: TenantId::new("a.b").expect("valid"),
                namespace: Namespace::new("c").expect("valid"),
                key: key("same"),
                kind: MemoryKind::Factual,
                content: "second pair".to_owned(),
                tags: Vec::new(),
                metadata: BTreeMap::new(),
                provenance: None,
            }
            .validated()
            .expect("valid"),
        )
        .expect("put");

    let first = store
        .get(
            &TenantId::new("a").expect("valid"),
            &Namespace::new("b.c").expect("valid"),
            &key("same"),
        )
        .expect("get")
        .expect("present");
    assert_eq!(
        first.content, "first pair",
        "a separator inside an identifier must not merge two partitions"
    );
}

/// A store that is shared across threads must keep each tenant's data its own.
fn concurrent_writers_do_not_cross_tenants(store: &std::sync::Arc<dyn MemoryStore>) {
    const THREADS: u32 = 8;
    const PER_THREAD: u32 = 25;

    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let store = std::sync::Arc::clone(store);
            scope.spawn(move || {
                let owner = format!("t-{thread}");
                for index in 0..PER_THREAD {
                    store
                        .put(record(
                            &owner,
                            "concurrent",
                            &format!("k-{index}"),
                            MemoryKind::Factual,
                            "content",
                            &[],
                            Some(u64::from(index)),
                        ))
                        .expect("put succeeds under contention");
                }
            });
        }
    });

    for thread in 0..THREADS {
        let found = store
            .query(
                &tenant(&format!("t-{thread}")),
                &namespace("concurrent"),
                &MemoryQuery::all(memory::MAX_QUERY_LIMIT).expect("valid"),
            )
            .expect("query");
        assert_eq!(
            found.len(),
            PER_THREAD as usize,
            "each tenant must see exactly its own writes"
        );
    }
}

/// The port must stay usable behind a trait object, which is what makes
/// configuration-driven selection possible at all. This is a compile-time
/// assertion as much as a runtime one.
fn the_port_is_object_safe(store: &std::sync::Arc<dyn MemoryStore>) {
    let as_dyn: &dyn MemoryStore = store.as_ref();
    assert!(!as_dyn.guarantees().durable_across_restart);
    // Exercise the blanket impl on Arc, so a future generic method on the trait
    // breaks here rather than in a downstream binary.
    assert_eq!(store.guarantees(), as_dyn.guarantees());
}

#[test]
fn in_process_store_honours_the_stricter_clauses() {
    let store = memory::local::InProcessStore::new();
    ingress_validation_is_the_adapters_own_job(&store);
    a_failed_write_preserves_the_previous_record(&store);
    a_limit_counts_matches_rather_than_records_examined(&store);
    metadata_and_tags_round_trip(&store);
    every_kind_round_trips(&store);
    distinct_tenant_and_namespace_pairs_never_alias(&store);
    // Explicitly typed, so these two also assert that the concrete adapter
    // coerces into a trait object at all.
    let shared: std::sync::Arc<dyn MemoryStore> =
        std::sync::Arc::new(memory::local::InProcessStore::new());
    concurrent_writers_do_not_cross_tenants(&shared);
    the_port_is_object_safe(&shared);
}

#[cfg(feature = "agentic")]
#[test]
fn agentic_memory_store_honours_the_stricter_clauses() {
    use memory::agentic::AgenticMemoryInProcessStore;

    let store = AgenticMemoryInProcessStore::new();
    ingress_validation_is_the_adapters_own_job(&store);
    a_failed_write_preserves_the_previous_record(&store);
    a_limit_counts_matches_rather_than_records_examined(&store);
    metadata_and_tags_round_trip(&store);
    every_kind_round_trips(&store);
    distinct_tenant_and_namespace_pairs_never_alias(&store);
    let shared: std::sync::Arc<dyn MemoryStore> =
        std::sync::Arc::new(AgenticMemoryInProcessStore::new());
    concurrent_writers_do_not_cross_tenants(&shared);
    the_port_is_object_safe(&shared);
}

/// The two adapters must agree on refusals and on ordering, not only on the happy
/// path. The earlier agreement test sorted both result sets, which discarded the
/// key ordering the agentic adapter documents, and never exercised an input an
/// adapter could refuse.
#[cfg(feature = "agentic")]
#[test]
fn both_adapters_agree_on_refusals_and_ordering() {
    let local = memory::local::InProcessStore::new();
    let agentic = memory::agentic::AgenticMemoryInProcessStore::new();

    let oversized = MemoryRecord {
        tenant_id: tenant("acme"),
        namespace: namespace("agree"),
        key: key("big"),
        kind: MemoryKind::Factual,
        content: "a".repeat(memory::MAX_CONTENT_BYTES + 1),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        provenance: None,
    };
    assert_eq!(
        local.put(oversized.clone()),
        agentic.put(oversized),
        "the two adapters must refuse identical input identically"
    );

    for index in 0..4_u32 {
        let fixture = record(
            "acme",
            "agree",
            &format!("k-{index}"),
            MemoryKind::Factual,
            "content",
            &[],
            Some(u64::from(index)),
        );
        local.put(fixture.clone()).expect("local put");
        agentic.put(fixture).expect("agentic put");
    }

    let query = MemoryQuery::all(64).expect("valid");
    // Compared without sorting: ordering is part of what the adapters promise.
    assert_eq!(
        local
            .query(&tenant("acme"), &namespace("agree"), &query)
            .expect("local query"),
        agentic
            .query(&tenant("acme"), &namespace("agree"), &query)
            .expect("agentic query"),
        "both adapters must return the same records in the same order"
    );

    assert_eq!(
        local
            .delete(&tenant("acme"), &namespace("agree"), &key("k-0"))
            .expect("local delete"),
        agentic
            .delete(&tenant("acme"), &namespace("agree"), &key("k-0"))
            .expect("agentic delete"),
    );
    assert_eq!(
        local
            .delete(&tenant("acme"), &namespace("agree"), &key("absent"))
            .expect("local delete"),
        agentic
            .delete(&tenant("acme"), &namespace("agree"), &key("absent"))
            .expect("agentic delete"),
        "both must report an absent delete the same way"
    );
    assert_eq!(
        local
            .get(&tenant("acme"), &namespace("agree"), &key("absent"))
            .expect("local get"),
        agentic
            .get(&tenant("acme"), &namespace("agree"), &key("absent"))
            .expect("agentic get"),
    );
}

// ---------------------------------------------------------------------------
// Clause 6 (queries) and clause 8 (capacity).
//
// Added after a security review found that per-request limits bounded the cost of
// one call and nothing else: a caller making only valid requests could grow the
// store without limit, and a caller holding the port directly could pass
// `limit: u32::MAX` and bypass every result ceiling in the brick.
// ---------------------------------------------------------------------------

/// Clause 6: an adapter validates a query itself, not just a record.
fn an_adapter_refuses_an_invalid_query_on_its_own<S: MemoryStore>(store: &S) {
    // Built by struct literal rather than `MemoryQuery::all`, which is exactly
    // what a caller holding the port directly can do.
    let unbounded = MemoryQuery {
        limit: u32::MAX,
        ..MemoryQuery::default()
    };
    assert_eq!(
        store.query(&tenant("acme"), &namespace("any"), &unbounded),
        Err(memory::MemoryError::LimitExceeded),
        "an adapter must not honour a limit above the core ceiling"
    );

    let zero = MemoryQuery::default();
    assert_eq!(
        store.query(&tenant("acme"), &namespace("any"), &zero),
        Err(memory::MemoryError::LimitExceeded),
        "a zero limit is refused rather than treated as unlimited"
    );

    let inverted = MemoryQuery {
        limit: 8,
        since: Some(Timestamp::from_micros(10)),
        until: Some(Timestamp::from_micros(5)),
        ..MemoryQuery::default()
    };
    assert_eq!(
        store.query(&tenant("acme"), &namespace("any"), &inverted),
        Err(memory::MemoryError::InvalidQuery),
        "an adapter applies the same cross-field rules as the service"
    );
}

/// Clause 8: a partition refuses growth past its ceiling, and stays updatable.
fn a_partition_is_capacity_bounded<S: MemoryStore>(store: &S) {
    let space = "capacity";
    for index in 0..memory::MAX_PARTITION_RECORDS {
        store
            .put(record(
                "capped",
                space,
                &format!("k-{index}"),
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("writes up to the ceiling succeed");
    }
    assert_eq!(
        store.put(record(
            "capped",
            space,
            "one-too-many",
            MemoryKind::Factual,
            "content",
            &[],
            None,
        )),
        Err(memory::MemoryError::LimitExceeded),
        "a new key past the ceiling is refused"
    );

    // A full partition must stay updatable, or it could never be repaired.
    assert_eq!(
        store
            .put(record(
                "capped",
                space,
                "k-0",
                MemoryKind::Factual,
                "updated content",
                &[],
                None,
            ))
            .expect("a replace consumes no capacity"),
        WriteOutcome::Replaced,
        "replacing an existing key must always be allowed"
    );

    // And deleting must free capacity again.
    assert!(
        store
            .delete(&tenant("capped"), &namespace(space), &key("k-1"))
            .expect("delete"),
    );
    assert_eq!(
        store
            .put(record(
                "capped",
                space,
                "after-delete",
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("capacity was freed"),
        WriteOutcome::Created,
    );
}

/// Clause 8: a tenant cannot mint unbounded namespaces to sidestep the record cap.
fn a_tenant_is_namespace_bounded<S: MemoryStore>(store: &S) {
    for index in 0..memory::MAX_TENANT_NAMESPACES {
        store
            .put(record(
                "spreader",
                &format!("ns-{index}"),
                "k",
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("namespaces up to the ceiling succeed");
    }
    assert_eq!(
        store.put(record(
            "spreader",
            "one-namespace-too-many",
            "k",
            MemoryKind::Factual,
            "content",
            &[],
            None,
        )),
        Err(memory::MemoryError::LimitExceeded),
        "a new namespace past the ceiling is refused"
    );
    // An existing namespace must still accept writes.
    assert_eq!(
        store
            .put(record(
                "spreader",
                "ns-0",
                "another-key",
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("an existing namespace is unaffected"),
        WriteOutcome::Created,
    );
    // The ceiling is per tenant, so another tenant is unaffected by this one
    // having filled its own allowance.
    assert_eq!(
        store
            .put(record(
                "unaffected",
                "fresh",
                "k",
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("another tenant has its own allowance"),
        WriteOutcome::Created,
    );
}

/// A capacity refusal must not have changed anything.
fn a_refused_write_leaves_the_store_unchanged<S: MemoryStore>(store: &S) {
    let space = "unchanged";
    for index in 0..memory::MAX_PARTITION_RECORDS {
        store
            .put(record(
                "full",
                space,
                &format!("k-{index}"),
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("fill to the ceiling");
    }
    assert!(
        store
            .put(record(
                "full",
                space,
                "rejected",
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .is_err()
    );
    assert!(
        store
            .get(&tenant("full"), &namespace(space), &key("rejected"))
            .expect("get")
            .is_none(),
        "a refused write must not have stored anything"
    );
    let still_there = store
        .query(
            &tenant("full"),
            &namespace(space),
            &MemoryQuery::all(memory::MAX_QUERY_LIMIT).expect("valid"),
        )
        .expect("query");
    assert_eq!(
        still_there.len(),
        memory::MAX_QUERY_LIMIT as usize,
        "the existing records are untouched"
    );
}

/// A foreign tenant writing a key that exists for someone else must observe
/// `Created`, or the outcome becomes an existence oracle.
fn a_write_outcome_is_not_an_existence_oracle<S: MemoryStore>(store: &S) {
    store
        .put(record(
            "owner",
            "oracle",
            "contested",
            MemoryKind::Factual,
            "owner's content",
            &[],
            None,
        ))
        .expect("put");
    assert_eq!(
        store
            .put(record(
                "stranger",
                "oracle",
                "contested",
                MemoryKind::Factual,
                "stranger's content",
                &[],
                None,
            ))
            .expect("put"),
        WriteOutcome::Created,
        "a foreign tenant must not learn that the key exists elsewhere"
    );
    assert!(
        !store
            .delete(&tenant("stranger"), &namespace("oracle"), &key("absent"))
            .expect("delete"),
        "a delete of an absent key reports the same regardless of other tenants"
    );
}

/// Deleting every record must release the partition, or namespace churn
/// accumulates permanently against the namespace ceiling.
fn an_emptied_partition_is_reclaimed<S: MemoryStore>(store: &S) {
    // Churn through more namespaces than the ceiling allows, emptying each one.
    // Without reclamation the ceiling would be hit partway through.
    for index in 0..(memory::MAX_TENANT_NAMESPACES * 2) {
        let space = format!("churn-{index}");
        store
            .put(record(
                "recycler",
                &space,
                "k",
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("a reclaimed namespace does not count against the ceiling");
        assert!(
            store
                .delete(&tenant("recycler"), &namespace(&space), &key("k"))
                .expect("delete"),
        );
    }
}

#[test]
fn in_process_store_is_bounded() {
    let store = memory::local::InProcessStore::new();
    an_adapter_refuses_an_invalid_query_on_its_own(&store);
    a_partition_is_capacity_bounded(&store);
    a_tenant_is_namespace_bounded(&store);
    a_write_outcome_is_not_an_existence_oracle(&store);
    an_emptied_partition_is_reclaimed(&store);
    a_refused_write_leaves_the_store_unchanged(&memory::local::InProcessStore::new());
}

#[cfg(feature = "agentic")]
#[test]
fn agentic_memory_store_is_bounded() {
    use memory::agentic::AgenticMemoryInProcessStore;

    let store = AgenticMemoryInProcessStore::new();
    an_adapter_refuses_an_invalid_query_on_its_own(&store);
    a_partition_is_capacity_bounded(&store);
    a_tenant_is_namespace_bounded(&store);
    a_write_outcome_is_not_an_existence_oracle(&store);
    an_emptied_partition_is_reclaimed(&store);
    a_refused_write_leaves_the_store_unchanged(&AgenticMemoryInProcessStore::new());
}

/// A query must stay cheap after the writes that break the vendor's fast path.
///
/// The agentic adapter's backend degrades `get_node` to a linear scan once a node
/// has been removed, which its own replace path does. Driving that per key made a
/// query quadratic in partition size. This runs a full partition through replaces
/// and then queries with a filter that matches nothing, which is the worst case:
/// no early exit. It is a timeout guard, not a benchmark — the point is that it
/// completes at all.
#[cfg(feature = "agentic")]
#[test]
fn a_query_stays_tractable_after_replaces() {
    use std::time::Instant;

    let store = memory::agentic::AgenticMemoryInProcessStore::new();
    let space = "tractable";
    for index in 0..memory::MAX_PARTITION_RECORDS {
        store
            .put(record(
                "acme",
                space,
                &format!("k-{index}"),
                MemoryKind::Factual,
                "content",
                &[],
                None,
            ))
            .expect("put");
    }
    // Break the backend's constant-time lookup.
    for index in 0..64 {
        store
            .put(record(
                "acme",
                space,
                &format!("k-{index}"),
                MemoryKind::Factual,
                "replaced content",
                &[],
                None,
            ))
            .expect("replace");
    }

    let mut query = MemoryQuery::all(memory::MAX_QUERY_LIMIT).expect("valid");
    query.term = Some("no-such-term".to_owned());
    let started = Instant::now();
    for _ in 0..20 {
        assert!(
            store
                .query(&tenant("acme"), &namespace(space), &query)
                .expect("query")
                .is_empty()
        );
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_secs() < 10,
        "20 non-matching queries over a full partition took {elapsed:?}, \
         which suggests the per-key lookup is back on the hot path"
    );
}
