use std::sync::Arc;

use storage::{
    DeleteCondition, DeleteOutcome, ListLimit, ListPage, ListRequest, MAX_LIST_LIMIT,
    MAX_NAMESPACE_BYTES, MAX_OBJECT_KEY_BYTES, MAX_OBJECT_VALUE_BYTES, MAX_OBJECTS_GLOBAL,
    MAX_OBJECTS_PER_TENANT, MAX_TENANT_ID_BYTES, MAX_VALUE_BYTES_GLOBAL,
    MAX_VALUE_BYTES_PER_TENANT, Namespace, ObjectKey, ObjectStore, ObjectValue,
    PersistenceGuarantee, PutCondition, PutOutcome, StorageError, StorageLimits, StorageScope,
    StoreGuarantees, StoredObject, TenantId,
};

fn assert_send_sync<T: Send + Sync>() {}
fn assert_clone_eq<T: Clone + Eq>() {}

#[test]
fn fixed_public_constants_match_v1() {
    assert_eq!(MAX_TENANT_ID_BYTES, 128_u16);
    assert_eq!(MAX_NAMESPACE_BYTES, 128_u16);
    assert_eq!(MAX_OBJECT_KEY_BYTES, 1_024_u16);
    assert_eq!(MAX_OBJECT_VALUE_BYTES, 1_048_576_u32);
    assert_eq!(MAX_LIST_LIMIT, 1_000_u32);
    assert_eq!(MAX_OBJECTS_PER_TENANT, 100_000_u64);
    assert_eq!(MAX_VALUE_BYTES_PER_TENANT, 1_073_741_824_u64);
    assert_eq!(MAX_OBJECTS_GLOBAL, 1_000_000_u64);
    assert_eq!(MAX_VALUE_BYTES_GLOBAL, 8_589_934_592_u64);
}

#[test]
fn constructors_enforce_exact_boundaries_and_arbitrary_key_bytes() {
    for valid in ["A", "a.b_c-9", &"x".repeat(128)] {
        assert!(
            TenantId::new(valid).is_ok(),
            "tenant boundary must accept {valid:?}"
        );
        assert!(
            Namespace::new(valid).is_ok(),
            "namespace boundary must accept {valid:?}"
        );
    }
    for invalid in ["", ".abc", "-abc", "a/b", "é", &"x".repeat(129)] {
        assert_eq!(TenantId::new(invalid), Err(StorageError::InvalidTenantId));
        assert_eq!(Namespace::new(invalid), Err(StorageError::InvalidNamespace));
    }

    assert_eq!(
        ObjectKey::new(Vec::<u8>::new()),
        Err(StorageError::InvalidObjectKey)
    );
    assert!(ObjectKey::new(vec![0]).is_ok());
    assert!(ObjectKey::new(vec![0xff; 1_024]).is_ok());
    assert_eq!(
        ObjectKey::new(vec![0; 1_025]),
        Err(StorageError::InvalidObjectKey)
    );

    assert!(ObjectValue::new(Vec::<u8>::new()).is_ok());
    assert!(ObjectValue::new(vec![0xa5; 1_048_576]).is_ok());
    assert_eq!(
        ObjectValue::new(vec![0; 1_048_577]),
        Err(StorageError::InvalidValue)
    );

    assert_eq!(ListLimit::new(0), Err(StorageError::InvalidListLimit));
    assert_eq!(ListLimit::new(1).expect("minimum limit").get(), 1);
    assert_eq!(ListLimit::new(1_000).expect("maximum limit").get(), 1_000);
    assert_eq!(ListLimit::new(1_001), Err(StorageError::InvalidListLimit));
}

#[test]
fn scope_request_and_outcome_shapes_are_publicly_constructible() {
    let scope = StorageScope {
        tenant_id: TenantId::new("tenant").expect("valid tenant"),
        namespace: Namespace::new("namespace").expect("valid namespace"),
    };
    let cursor = ObjectKey::new(vec![0, 255]).expect("valid arbitrary cursor");
    let request = ListRequest {
        after_key: Some(cursor.clone()),
        limit: ListLimit::new(7).expect("valid limit"),
    };
    assert_eq!(
        scope,
        StorageScope::new(scope.tenant_id.clone(), scope.namespace.clone())
    );
    assert_eq!(request, ListRequest::new(Some(cursor), request.limit));

    let page = ListPage {
        objects: vec![],
        has_more: false,
    };
    assert!(page.objects.is_empty());
    assert!(!page.has_more);
}

#[test]
fn object_version_is_opaque_and_debug_is_non_revealing() {
    assert_clone_eq::<storage::ObjectVersion>();
    let limits = StorageLimits::new(1, 1, 1, 1).expect("valid limits");
    #[cfg(feature = "local")]
    {
        let store = storage::local::LocalObjectStore::new(limits);
        let outcome = store
            .put(
                &StorageScope::new(TenantId::new("t").unwrap(), Namespace::new("n").unwrap()),
                ObjectKey::new(b"k".to_vec()).unwrap(),
                ObjectValue::new(Vec::new()).unwrap(),
                PutCondition::Any,
            )
            .unwrap();
        let PutOutcome::Created { version } = outcome else {
            panic!("expected create")
        };
        assert_eq!(format!("{version:?}"), "ObjectVersion(<opaque>)");
        assert!(
            !format!("{version:?}")
                .chars()
                .any(|character| character.is_ascii_digit())
        );
    }
    #[cfg(not(feature = "local"))]
    let _ = limits;
}

#[test]
fn errors_have_exact_codes_and_redacted_display_and_debug() {
    let cases = [
        (StorageError::InvalidTenantId, "invalid_tenant_id"),
        (StorageError::InvalidNamespace, "invalid_namespace"),
        (StorageError::InvalidObjectKey, "invalid_object_key"),
        (StorageError::InvalidValue, "invalid_value"),
        (StorageError::InvalidListLimit, "invalid_list_limit"),
        (StorageError::InvalidLimits, "invalid_limits"),
        (StorageError::LimitExceeded, "limit_exceeded"),
        (StorageError::RevisionExhausted, "revision_exhausted"),
        (StorageError::LockUnavailable, "lock_unavailable"),
        (StorageError::CorruptStore, "corrupt_store"),
        (StorageError::OperationFailed, "operation_failed"),
    ];
    for (error, code) in cases {
        assert_eq!(error.code(), code);
        assert_eq!(error.to_string(), code);
        assert_eq!(format!("{error:?}"), code);
    }
}

#[test]
fn limits_and_guarantee_types_have_exact_fixed_width_shape() {
    let limits = StorageLimits::new(2, 3, 4, 5).expect("valid limits");
    let _: u64 = limits.max_objects_per_tenant();
    let _: u64 = limits.max_value_bytes_per_tenant();
    let _: u64 = limits.max_objects_global();
    let _: u64 = limits.max_value_bytes_global();
    assert_eq!(limits.max_objects_per_tenant(), 2);

    let guarantees = StoreGuarantees {
        persistence: PersistenceGuarantee::CleanRestart,
        shared_across_processes: false,
        per_operation_atomic: true,
        conditional_writes: true,
        eviction: false,
        limits,
    };
    assert_eq!(guarantees.persistence, PersistenceGuarantee::CleanRestart);
    assert!(!guarantees.shared_across_processes);
    assert!(guarantees.per_operation_atomic);
    assert!(guarantees.conditional_writes);
    assert!(!guarantees.eviction);
}

#[allow(dead_code, clippy::too_many_arguments)]
fn compile_exact_five_method_contract(
    store: &dyn ObjectStore,
    scope: &StorageScope,
    put_key: ObjectKey,
    delete_key: &ObjectKey,
    value: ObjectValue,
    put_condition: PutCondition,
    delete_condition: DeleteCondition,
    request: &ListRequest,
) {
    let _: Result<Option<StoredObject>, StorageError> = store.get(scope, delete_key);
    let _: Result<PutOutcome, StorageError> = store.put(scope, put_key, value, put_condition);
    let _: Result<DeleteOutcome, StorageError> = store.delete(scope, delete_key, delete_condition);
    let _: Result<ListPage, StorageError> = store.list(scope, request);
    let _: StoreGuarantees = store.guarantees();
}

#[test]
fn object_store_is_send_sync_object_safe_and_has_exact_five_method_signatures() {
    assert_send_sync::<Arc<dyn ObjectStore>>();
}

#[test]
fn source_surface_has_only_five_port_methods_and_no_numeric_version_api() {
    let port_source = include_str!("../src/port.rs");
    let trait_body = port_source
        .split("pub trait ObjectStore")
        .nth(1)
        .expect("ObjectStore trait exists")
        .split("\n}\n\nimpl")
        .next()
        .expect("ObjectStore trait body");
    assert_eq!(trait_body.matches("    fn ").count(), 5);
    for method in [
        "fn get(",
        "fn put(",
        "fn delete(",
        "fn list(",
        "fn guarantees(",
    ] {
        assert_eq!(
            trait_body.matches(method).count(),
            1,
            "exact method {method}"
        );
    }

    let model_source = include_str!("../src/model.rs");
    let version_body = model_source
        .split("impl ObjectVersion")
        .nth(1)
        .expect("ObjectVersion implementation exists")
        .split("\n}\n\nimpl fmt::Debug")
        .next()
        .expect("ObjectVersion implementation body");
    assert!(!version_body.contains("\n    pub fn "));
    assert!(!version_body.contains("\n    pub const fn "));
    assert!(!model_source.contains("impl fmt::Display for ObjectVersion"));
    assert!(!model_source.contains("impl Ord for ObjectVersion"));
    assert!(!model_source.contains("impl PartialOrd for ObjectVersion"));
}
