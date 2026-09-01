//! Adversarial integration tests for `DockerSandbox` against a fake `docker`
//! CLI (`tests/fixtures/fake_docker.py`). These exercise the deterministic
//! `derived_id` + `inspect_owned` idempotent-replay path end to end, without
//! requiring a real Docker daemon.
#![cfg(feature = "docker")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicU64, Ordering};

use sandbox::docker::DockerSandbox;
use sandbox::{
    CorrelationId, DockerProfile, PrincipalId, ProfileId, RequestId, Sandbox, SandboxContext,
    SandboxError, SandboxStatus, StartRequest, TargetRequest, TenantId,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_docker.py")
}

/// Fake-docker state is keyed off the `--host unix://<socket>` argument (the
/// real CLI is spawned with `.env_clear()`, so env vars cannot reach it).
/// Returns a fresh, never-before-used socket path so each test gets isolated
/// container state; state/lock/delay files live alongside it.
fn fresh_socket_path() -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sandbox-fake-docker-{}-{unique}.sock",
        std::process::id()
    ))
}

fn sibling(socket: &Path, suffix: &str) -> PathBuf {
    let mut path = socket.to_path_buf().into_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn read_state(socket: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(sibling(socket, ".state.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn write_state(socket: &Path, value: &serde_json::Value) {
    std::fs::write(sibling(socket, ".state.json"), value.to_string()).unwrap();
}

fn set_run_delay_ms(socket: &Path, millis: u64) {
    std::fs::write(sibling(socket, ".delay"), millis.to_string()).unwrap();
}

fn cleanup(socket: &Path) {
    for suffix in [".state.json", ".state.json.lock", ".delay"] {
        let _ = std::fs::remove_file(sibling(socket, suffix));
    }
}

fn owner(tag: &str) -> String {
    // 64 lowercase-hex chars, as required by DockerSandbox::new.
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut hash);
    format!("{:016x}", hash.finish()).repeat(4)[..64].to_string()
}

fn profile() -> (ProfileId, DockerProfile) {
    let id = ProfileId::new("rust").unwrap();
    let value = DockerProfile::new(
        format!("rust@sha256:{}", "a".repeat(64)),
        "1000",
        64 * 1024 * 1024,
        100,
        16,
    )
    .unwrap();
    (id, value)
}

/// Builds a `DockerSandbox` wired to the fake CLI, using `socket_path` as the
/// `--host unix://` endpoint. Each distinct socket path gets isolated
/// container state in the fake CLI.
fn adapter_with_socket(owner_hex: &str, socket_path: &Path) -> DockerSandbox {
    let (profile_id, profile_value) = profile();
    DockerSandbox::new(
        fixture_path(),
        format!("unix://{}", socket_path.display()),
        owner_hex,
        BTreeMap::from([(profile_id, profile_value)]),
    )
    .unwrap()
}

fn context(tenant: &str, request: &str, correlation: &str) -> SandboxContext {
    SandboxContext {
        tenant_id: TenantId::new(tenant).unwrap(),
        principal_id: PrincipalId::new("principal").unwrap(),
        request_id: RequestId::new(request).unwrap(),
        correlation_id: CorrelationId::new(correlation).unwrap(),
    }
}

fn external_stop(socket_path: &Path, name: &str) {
    let output = StdCommand::new(fixture_path())
        .args([
            "--host",
            &format!("unix://{}", socket_path.display()),
            "stopext",
            name,
        ])
        .output()
        .expect("fake docker invocation failed");
    assert!(output.status.success(), "stopext failed: {output:?}");
}

// --- 1. Retry / idempotency correctness ------------------------------------

#[test]
fn identical_start_request_is_idempotent_and_returns_same_id() {
    let socket = fresh_socket_path();
    let hex = owner("idempotent");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();
    let request = || StartRequest {
        context: context("tenant-a", "req-1", "corr-1"),
        profile_id: profile_id.clone(),
    };

    let first = adapter.start(request()).unwrap();
    assert_eq!(first.status, SandboxStatus::Running);

    let second = adapter.start(request()).unwrap();
    assert_eq!(
        second.sandbox_id, first.sandbox_id,
        "retry of an identical StartRequest must derive the same SandboxId"
    );
    assert_eq!(
        second.status,
        SandboxStatus::Running,
        "retry against a still-running container must report Running, not attempt a new run"
    );

    cleanup(&socket);
}

#[test]
fn retry_after_external_stop_reports_stopped_without_erroring_or_duplicating() {
    let socket = fresh_socket_path();
    let hex = owner("idempotent-stopped");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();
    let request = || StartRequest {
        context: context("tenant-b", "req-2", "corr-2"),
        profile_id: profile_id.clone(),
    };

    let first = adapter.start(request()).unwrap();
    let name = format!("rf-{}", first.sandbox_id.as_str());
    external_stop(&socket, &name);

    let second = adapter.start(request()).unwrap();
    assert_eq!(
        second.sandbox_id, first.sandbox_id,
        "SandboxId derivation must stay stable even though external state changed"
    );
    assert_eq!(
        second.status,
        SandboxStatus::Stopped,
        "start must report the container's *current* status (Stopped), not assume Running \
         and not attempt to recreate/duplicate the container"
    );

    cleanup(&socket);
}

#[test]
fn three_sequential_retries_never_create_more_than_one_container() {
    let socket = fresh_socket_path();
    let hex = owner("idempotent-triple");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();
    let request = || StartRequest {
        context: context("tenant-c", "req-3", "corr-3"),
        profile_id: profile_id.clone(),
    };

    let ids: Vec<_> = (0..3).map(|_| adapter.start(request()).unwrap()).collect();
    let unique_ids: std::collections::BTreeSet<_> = ids
        .iter()
        .map(|result| result.sandbox_id.as_str().to_owned())
        .collect();
    assert_eq!(
        unique_ids.len(),
        1,
        "all three retries must resolve to one SandboxId"
    );

    let parsed = read_state(&socket);
    assert_eq!(
        parsed.as_object().unwrap().len(),
        1,
        "the fake docker daemon's container table must contain exactly one entry after 3 retries"
    );

    cleanup(&socket);
}

// --- 2. Cross-tenant / cross-owner collision --------------------------------

#[test]
fn different_tenant_same_owner_never_collides_because_id_is_tenant_scoped() {
    let socket = fresh_socket_path();
    let hex = owner("cross-tenant");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();

    let a = adapter
        .start(StartRequest {
            context: context("tenant-a", "req-x", "corr-x"),
            profile_id: profile_id.clone(),
        })
        .unwrap();
    let b = adapter
        .start(StartRequest {
            context: context("tenant-b", "req-x", "corr-x"),
            profile_id: profile_id.clone(),
        })
        .unwrap();

    assert_ne!(
        a.sandbox_id, b.sandbox_id,
        "identical request/correlation ids under different tenants must derive different \
         SandboxIds, so there is no name to collide on"
    );

    cleanup(&socket);
}

#[test]
fn forged_label_mismatch_on_existing_name_is_denied_not_reused() {
    // Simulates a name collision where the *label metadata* on an existing
    // container does not match the caller's tenant (e.g. state corruption,
    // or a differently-derived owner keyspace landing on the same name).
    // inspect_owned's field validation must deny rather than hand back or
    // overwrite the container.
    let socket = fresh_socket_path();
    let hex = owner("collision-owner");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();

    let started = adapter
        .start(StartRequest {
            context: context("tenant-real", "req-y", "corr-y"),
            profile_id: profile_id.clone(),
        })
        .unwrap();
    let name = format!("rf-{}", started.sandbox_id.as_str());

    let mut parsed = read_state(&socket);
    parsed[&name]["tenant"] = serde_json::Value::String("someone-else".into());
    write_state(&socket, &parsed);

    let target = TargetRequest {
        context: context("tenant-real", "req-y", "corr-y"),
        sandbox_id: started.sandbox_id.clone(),
    };
    let result = adapter.status(target);
    assert_eq!(
        result,
        Err(SandboxError::Denied),
        "a name whose live labels don't match the caller's identity must be denied, not \
         silently reused or reported as owned"
    );

    cleanup(&socket);
}

#[test]
fn start_on_label_mismatched_existing_name_is_denied_and_leaves_container_untouched() {
    // Same setup as above, but through `start` itself: the retry path must
    // deny rather than attempt `docker run` (which would fail on duplicate
    // name anyway) or treat the foreign container as a safe no-op.
    let socket = fresh_socket_path();
    let hex = owner("collision-owner-2");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();

    let started = adapter
        .start(StartRequest {
            context: context("tenant-real2", "req-z", "corr-z"),
            profile_id: profile_id.clone(),
        })
        .unwrap();
    let name = format!("rf-{}", started.sandbox_id.as_str());

    let mut parsed = read_state(&socket);
    let original_docker_id = parsed[&name]["id"].as_str().unwrap().to_owned();
    parsed[&name]["owner"] = serde_json::Value::String("f".repeat(64));
    write_state(&socket, &parsed);

    let retry = adapter.start(StartRequest {
        context: context("tenant-real2", "req-z", "corr-z"),
        profile_id: profile_id.clone(),
    });
    assert_eq!(retry, Err(SandboxError::Denied));

    // The foreign/mismatched container must be left exactly as-is: start must
    // not remove, replace, or otherwise mutate a name it doesn't own.
    let parsed_after = read_state(&socket);
    assert_eq!(
        parsed_after[&name]["id"].as_str().unwrap(),
        original_docker_id,
        "start must not remove or overwrite a container it does not own"
    );

    cleanup(&socket);
}

// --- 4. Lifecycle regression (execute/status/stop unaffected) --------------

#[test]
fn full_lifecycle_start_execute_status_stop_against_fake_daemon() {
    let socket = fresh_socket_path();
    let hex = owner("lifecycle");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();
    let context_value = context("tenant-lifecycle", "req-full", "corr-full");

    let started = adapter
        .start(StartRequest {
            context: context_value.clone(),
            profile_id,
        })
        .unwrap();
    assert_eq!(started.status, SandboxStatus::Running);

    let executed = adapter
        .execute(sandbox::ExecuteRequest {
            context: context_value.clone(),
            sandbox_id: started.sandbox_id.clone(),
            command: sandbox::Command::new("cargo", vec!["--version".into()], "", 1_000).unwrap(),
        })
        .unwrap();
    assert_eq!(executed.exit_code, 0);
    assert_eq!(executed.stdout, "exec-ok");

    let status = adapter
        .status(TargetRequest {
            context: context_value.clone(),
            sandbox_id: started.sandbox_id.clone(),
        })
        .unwrap();
    assert_eq!(status.status, SandboxStatus::Running);

    let stopped = adapter
        .stop(TargetRequest {
            context: context_value.clone(),
            sandbox_id: started.sandbox_id.clone(),
        })
        .unwrap();
    assert!(stopped.removed);

    let after_stop = adapter.status(TargetRequest {
        context: context_value,
        sandbox_id: started.sandbox_id,
    });
    assert_eq!(
        after_stop,
        Err(SandboxError::NotFound),
        "status after stop must report NotFound, matching pre-existing (unchanged) behavior"
    );

    cleanup(&socket);
}

#[test]
fn execute_on_stopped_container_reports_not_found_unchanged() {
    let socket = fresh_socket_path();
    let hex = owner("execute-stopped");
    let adapter = adapter_with_socket(&hex, &socket);
    let (profile_id, _) = profile();
    let context_value = context("tenant-exec-stopped", "req-es", "corr-es");

    let started = adapter
        .start(StartRequest {
            context: context_value.clone(),
            profile_id,
        })
        .unwrap();
    let name = format!("rf-{}", started.sandbox_id.as_str());
    external_stop(&socket, &name);

    let result = adapter.execute(sandbox::ExecuteRequest {
        context: context_value,
        sandbox_id: started.sandbox_id,
        command: sandbox::Command::new("cargo", vec![], "", 1_000).unwrap(),
    });
    assert_eq!(result, Err(SandboxError::NotFound));

    cleanup(&socket);
}

// --- 3. Race / concurrency (documented behavior check) ----------------------

#[test]
fn concurrent_identical_start_calls_do_not_duplicate_containers() {
    // This proves the *observable outcome* (no duplicate container survives)
    // using the fake daemon's atomic-under-lock `run --name` semantics, which
    // mirror a real Docker daemon's atomic name registration. It does not by
    // itself prove the Rust-side inspect-then-run window is race-free -- see
    // the Required finding in the QA report regarding an unavoidable
    // inspect/run TOCTOU when *no* container yet exists and two callers race.
    let socket = fresh_socket_path();
    // Force the `run` call to be slow so both threads are past `inspect_owned`
    // (which sees nothing) before either commits `run`.
    set_run_delay_ms(&socket, 200);
    let hex = owner("race");
    let adapter = std::sync::Arc::new(adapter_with_socket(&hex, &socket));
    let (profile_id, _) = profile();

    let request = || StartRequest {
        context: context("tenant-race", "req-race", "corr-race"),
        profile_id: profile_id.clone(),
    };

    let a = std::thread::spawn({
        let adapter = adapter.clone();
        let request = request();
        move || adapter.start(request)
    });
    let b = std::thread::spawn({
        let adapter = adapter.clone();
        let request = request();
        move || adapter.start(request)
    });

    let result_a = a.join().unwrap();
    let result_b = b.join().unwrap();

    // At least one side must succeed. The other must either succeed with the
    // same id (idempotent replay after the winner committed) or fail cleanly
    // (e.g. OperationFailed from a `run --name` conflict raced against
    // inspect) -- but there must never be two live containers.
    let outcomes = [&result_a, &result_b];
    let successes: Vec<_> = outcomes.iter().filter_map(|r| r.as_ref().ok()).collect();
    assert!(
        !successes.is_empty(),
        "at least one concurrent start must succeed"
    );

    let parsed = read_state(&socket);
    assert_eq!(
        parsed.as_object().unwrap().len(),
        1,
        "concurrent identical start calls must never leave more than one container behind \
         (found {} entries: {parsed})",
        parsed.as_object().unwrap().len()
    );

    cleanup(&socket);
}
