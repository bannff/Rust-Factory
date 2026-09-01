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
    assert_eq!(events.0.lock().unwrap().len(), 4);
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
