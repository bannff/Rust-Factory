use std::sync::{Arc, Mutex};

use proptest::prelude::*;
use sandbox::*;

fn context(tenant: &str) -> SandboxContext {
    SandboxContext {
        tenant_id: TenantId::new(tenant).unwrap(),
        principal_id: PrincipalId::new("principal").unwrap(),
        request_id: RequestId::new("request").unwrap(),
        correlation_id: CorrelationId::new("correlation").unwrap(),
    }
}
fn id() -> SandboxId {
    SandboxId::new(format!("sbx-{}", "a".repeat(32))).unwrap()
}

#[test]
fn tenant_id_equality_is_real_field_comparison_not_vacuous() {
    // Guards the tenant-comparison assertions used throughout this file's
    // #67 regression tests: PartialEq must genuinely distinguish different
    // tenant values, not be trivially true (e.g. via a discriminant-only or
    // always-true impl).
    assert_eq!(
        TenantId::new("tenant-alpha").unwrap(),
        TenantId::new("tenant-alpha").unwrap()
    );
    assert_ne!(
        TenantId::new("tenant-alpha").unwrap(),
        TenantId::new("tenant-beta").unwrap()
    );
    assert_ne!(TenantId::new("a").unwrap(), TenantId::new("b").unwrap());
}

#[test]
fn identifiers_and_sandbox_ids_are_closed() {
    for invalid in ["", "Upper", "has.dot", "has space", "-leading"] {
        assert_eq!(TenantId::new(invalid), Err(SandboxError::InvalidRequest));
    }
    assert!(TenantId::new("tenant-1").is_ok());
    assert!(SandboxId::new(format!("sbx-{}", "f".repeat(32))).is_ok());
    assert_eq!(
        SandboxId::new("container"),
        Err(SandboxError::InvalidRequest)
    );
}

#[test]
fn command_limits_and_paths_are_enforced() {
    assert!(Command::new("cargo", vec!["test".into()], "crate", 1).is_ok());
    assert_eq!(
        Command::new("", vec![], "", 1),
        Err(SandboxError::InvalidRequest)
    );
    assert_eq!(
        Command::new("cargo", vec![], "../host", 1),
        Err(SandboxError::InvalidRequest)
    );
    assert_eq!(
        Command::new("cargo", vec!["x".into(); MAX_ARGUMENTS + 1], "", 1),
        Err(SandboxError::LimitExceeded)
    );
    assert_eq!(
        Command::new("cargo", vec![], "", MAX_TIMEOUT_MILLIS + 1),
        Err(SandboxError::LimitExceeded)
    );
}

#[test]
fn docker_profiles_require_digest_non_root_and_bounds() {
    let image = format!("rust@sha256:{}", "a".repeat(64));
    assert!(DockerProfile::new(&image, "1000", 64 * 1024 * 1024, 100, 16).is_ok());
    for user in ["0", "root", "01000"] {
        assert_eq!(
            DockerProfile::new(&image, user, 64 * 1024 * 1024, 100, 16),
            Err(SandboxError::InvalidRequest)
        );
    }
    assert_eq!(
        DockerProfile::new("rust:latest", "1000", 64 * 1024 * 1024, 100, 16),
        Err(SandboxError::InvalidRequest)
    );
}

#[derive(Default)]
struct FakeSandbox;
impl Sandbox for FakeSandbox {
    fn start(&self, _request: StartRequest) -> Result<StartResult, SandboxError> {
        Ok(StartResult {
            sandbox_id: id(),
            status: SandboxStatus::Running,
        })
    }
    fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult, SandboxError> {
        Ok(ExecuteResult {
            sandbox_id: request.sandbox_id,
            exit_code: 0,
            stdout: "ok".into(),
            stderr: String::new(),
            truncated: false,
        })
    }
    fn status(&self, request: TargetRequest) -> Result<StatusResult, SandboxError> {
        Ok(StatusResult {
            sandbox_id: request.sandbox_id,
            status: SandboxStatus::Running,
        })
    }
    fn stop(&self, request: TargetRequest) -> Result<StopResult, SandboxError> {
        Ok(StopResult {
            sandbox_id: request.sandbox_id,
            removed: true,
        })
    }
}
#[derive(Default)]
struct Events(Mutex<Vec<SandboxEvent>>);
impl SandboxEventSink for Events {
    fn try_emit(&self, event: SandboxEvent) -> EventSubmission {
        self.0.lock().unwrap().push(event);
        EventSubmission::Accepted
    }
}
struct SharedEvents(Arc<Events>);
impl SandboxEventSink for SharedEvents {
    fn try_emit(&self, event: SandboxEvent) -> EventSubmission {
        self.0.try_emit(event)
    }
}

#[test]
fn service_delegates_four_operations_and_emits_safe_events() {
    let events = Arc::new(Events::default());
    let service = SandboxService::new(FakeSandbox, SharedEvents(events.clone()));
    let started = service
        .start(StartRequest {
            context: context("tenant"),
            profile_id: ProfileId::new("rust").unwrap(),
        })
        .unwrap();
    service
        .execute(ExecuteRequest {
            context: context("tenant"),
            sandbox_id: started.sandbox_id.clone(),
            command: Command::new("cargo", vec!["test".into()], "", 1000).unwrap(),
        })
        .unwrap();
    service
        .status(TargetRequest {
            context: context("tenant"),
            sandbox_id: started.sandbox_id.clone(),
        })
        .unwrap();
    service
        .stop(TargetRequest {
            context: context("tenant"),
            sandbox_id: started.sandbox_id,
        })
        .unwrap();
    let recorded = events.0.lock().unwrap();
    assert_eq!(recorded.len(), 4);
    for event in recorded.iter() {
        assert_eq!(
            event.tenant_id,
            TenantId::new("tenant").unwrap(),
            "every emitted event must carry the tenant of the request that produced it"
        );
    }
}

#[test]
fn emitted_events_carry_the_correct_tenant_per_call_not_a_fixed_or_stale_one() {
    // A single long-lived SandboxService/sink instance already serves many
    // tenants across calls (SandboxMcp::authorize resolves a fresh tenant
    // per call). Prove tenant_id on each emitted event matches THAT call's
    // context, not some value fixed at construction or leaked from a prior
    // call - regression proof for issue #67.
    let events = Arc::new(Events::default());
    let service = SandboxService::new(FakeSandbox, SharedEvents(events.clone()));

    let alpha = service
        .start(StartRequest {
            context: context("tenant-alpha"),
            profile_id: ProfileId::new("rust").unwrap(),
        })
        .unwrap();
    let beta = service
        .start(StartRequest {
            context: context("tenant-beta"),
            profile_id: ProfileId::new("rust").unwrap(),
        })
        .unwrap();
    service
        .stop(TargetRequest {
            context: context("tenant-alpha"),
            sandbox_id: alpha.sandbox_id,
        })
        .unwrap();
    service
        .stop(TargetRequest {
            context: context("tenant-beta"),
            sandbox_id: beta.sandbox_id,
        })
        .unwrap();

    let recorded = events.0.lock().unwrap();
    assert_eq!(recorded.len(), 4);
    assert_eq!(
        recorded[0].tenant_id,
        TenantId::new("tenant-alpha").unwrap()
    );
    assert_eq!(recorded[1].tenant_id, TenantId::new("tenant-beta").unwrap());
    assert_eq!(
        recorded[2].tenant_id,
        TenantId::new("tenant-alpha").unwrap()
    );
    assert_eq!(recorded[3].tenant_id, TenantId::new("tenant-beta").unwrap());
}

#[test]
fn emitted_events_carry_the_correct_tenant_across_all_four_interleaved_operations() {
    // The existing #67 regression test only interleaves start/stop. execute
    // and status share the identical `emit` call site in SandboxService, so
    // this exercises all four operations interleaved across two tenants on
    // one shared SandboxService instance, proving none of the four leak or
    // fix a tenant_id from another call.
    let events = Arc::new(Events::default());
    let service = SandboxService::new(FakeSandbox, SharedEvents(events.clone()));

    let alpha = service
        .start(StartRequest {
            context: context("tenant-alpha"),
            profile_id: ProfileId::new("rust").unwrap(),
        })
        .unwrap();
    let beta = service
        .start(StartRequest {
            context: context("tenant-beta"),
            profile_id: ProfileId::new("rust").unwrap(),
        })
        .unwrap();
    service
        .execute(ExecuteRequest {
            context: context("tenant-beta"),
            sandbox_id: beta.sandbox_id.clone(),
            command: Command::new("cargo", vec!["test".into()], "", 1000).unwrap(),
        })
        .unwrap();
    service
        .execute(ExecuteRequest {
            context: context("tenant-alpha"),
            sandbox_id: alpha.sandbox_id.clone(),
            command: Command::new("cargo", vec!["test".into()], "", 1000).unwrap(),
        })
        .unwrap();
    service
        .status(TargetRequest {
            context: context("tenant-alpha"),
            sandbox_id: alpha.sandbox_id.clone(),
        })
        .unwrap();
    service
        .status(TargetRequest {
            context: context("tenant-beta"),
            sandbox_id: beta.sandbox_id.clone(),
        })
        .unwrap();
    service
        .stop(TargetRequest {
            context: context("tenant-beta"),
            sandbox_id: beta.sandbox_id,
        })
        .unwrap();
    service
        .stop(TargetRequest {
            context: context("tenant-alpha"),
            sandbox_id: alpha.sandbox_id,
        })
        .unwrap();

    let recorded = events.0.lock().unwrap();
    assert_eq!(recorded.len(), 8);
    let expected = [
        (SandboxOperation::Start, "tenant-alpha"),
        (SandboxOperation::Start, "tenant-beta"),
        (SandboxOperation::Execute, "tenant-beta"),
        (SandboxOperation::Execute, "tenant-alpha"),
        (SandboxOperation::Status, "tenant-alpha"),
        (SandboxOperation::Status, "tenant-beta"),
        (SandboxOperation::Stop, "tenant-beta"),
        (SandboxOperation::Stop, "tenant-alpha"),
    ];
    for (event, (operation, tenant)) in recorded.iter().zip(expected.iter()) {
        assert_eq!(
            event.operation, *operation,
            "operation order must be preserved"
        );
        assert_eq!(
            event.tenant_id,
            TenantId::new(*tenant).unwrap(),
            "event for {operation:?} must carry the tenant of the call that produced it, \
             not a value leaked or fixed from another interleaved call"
        );
    }
}

#[test]
fn errors_are_stable_and_redacted() {
    for error in [
        SandboxError::InvalidRequest,
        SandboxError::NotFound,
        SandboxError::Denied,
        SandboxError::LimitExceeded,
        SandboxError::Timeout,
        SandboxError::Unavailable,
        SandboxError::OutcomeUnknown,
        SandboxError::OperationFailed,
    ] {
        assert_eq!(error.to_string(), error.as_str());
        assert!(!error.to_string().contains('/'));
    }
}

#[test]
fn ports_are_object_safe() {
    let value: Arc<dyn Sandbox> = Arc::new(FakeSandbox);
    let denied: Arc<dyn Sandbox> = Arc::new(DenySandbox);
    drop((value, denied));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn logical_id_acceptance_matches_grammar(bytes in prop::collection::vec(any::<u8>(), 0..150)) {
        let value = String::from_utf8_lossy(&bytes).into_owned();
        let raw = value.as_bytes();
        let expected = !raw.is_empty() && raw.len() <= MAX_ID_BYTES
            && (raw[0].is_ascii_lowercase() || raw[0].is_ascii_digit())
            && raw.iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_'));
        prop_assert_eq!(TenantId::new(value).is_ok(), expected);
    }
}
