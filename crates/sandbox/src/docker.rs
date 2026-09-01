use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use subprocess::{Exec, Redirection};

use crate::{
    DockerProfile, ExecuteRequest, ExecuteResult, MAX_OUTPUT_BYTES, Sandbox, SandboxError,
    SandboxId, SandboxStatus, StartRequest, StartResult, StatusResult, StopResult, TargetRequest,
};

const OWNER_LABEL: &str = "rust-factory.owner";
const TENANT_LABEL: &str = "rust-factory.tenant";
const SANDBOX_LABEL: &str = "rust-factory.sandbox";
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);

pub struct DockerSandbox {
    executable: PathBuf,
    endpoint: String,
    owner: String,
    profiles: BTreeMap<crate::ProfileId, DockerProfile>,
}

impl DockerSandbox {
    pub fn new(
        executable: impl Into<PathBuf>,
        endpoint: impl Into<String>,
        owner: impl Into<String>,
        profiles: BTreeMap<crate::ProfileId, DockerProfile>,
    ) -> Result<Self, SandboxError> {
        let executable = executable.into();
        let endpoint = endpoint.into();
        let owner = owner.into();
        let socket = endpoint
            .strip_prefix("unix://")
            .ok_or(SandboxError::InvalidRequest)?;
        let valid_owner = owner.len() == 64
            && owner
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !executable.is_absolute()
            || !Path::new(socket).is_absolute()
            || !valid_owner
            || profiles.is_empty()
        {
            return Err(SandboxError::InvalidRequest);
        }
        Ok(Self {
            executable,
            endpoint,
            owner,
            profiles,
        })
    }

    fn base_args(&self, operation: &str) -> Vec<String> {
        vec!["--host".into(), self.endpoint.clone(), operation.into()]
    }

    /// Derives a `SandboxId` deterministically from `(owner, tenant_id, request_id,
    /// correlation_id)`. Identical inputs always yield the identical ID within this
    /// adapter's lifetime, so a caller retrying an identical `StartRequest` maps onto
    /// the same container name instead of forking a duplicate. This is a request-identity
    /// derivation, not a durable idempotency key: the adapter holds no state across
    /// process restarts, and `start` still verifies live container ownership via
    /// `inspect_owned` before treating any name as safely reusable.
    fn derived_id(&self, request: &StartRequest) -> Result<SandboxId, SandboxError> {
        let mut hash = Sha256::new();
        hash.update(b"rust-factory:sandbox:v1\0");
        hash.update(self.owner.as_bytes());
        hash.update(b"\0");
        hash.update(request.context.tenant_id.as_str().as_bytes());
        hash.update(b"\0");
        hash.update(request.context.request_id.as_str().as_bytes());
        hash.update(b"\0");
        hash.update(request.context.correlation_id.as_str().as_bytes());
        SandboxId::new(format!("sbx-{}", &format!("{:x}", hash.finalize())[..32]))
    }

    fn name(id: &SandboxId) -> String {
        format!("rf-{}", id.as_str())
    }

    fn inspect_owned(
        &self,
        request: &TargetRequest,
    ) -> Result<Option<OwnedContainer>, SandboxError> {
        let mut args = self.base_args("inspect");
        args.extend([
            "--format".into(),
            format!(
                "{{{{.Id}}}}|{{{{index .Config.Labels \"{OWNER_LABEL}\"}}}}|{{{{index .Config.Labels \"{TENANT_LABEL}\"}}}}|{{{{index .Config.Labels \"{SANDBOX_LABEL}\"}}}}|{{{{.State.Running}}}}"
            ),
            Self::name(&request.sandbox_id),
        ]);
        let output = self.run_cli(&args, CONTROL_TIMEOUT)?;
        if output.code != 0 {
            return if confirmed_not_found(&output) {
                Ok(None)
            } else {
                Err(SandboxError::Unavailable)
            };
        }
        let fields = output.stdout.trim().split('|').collect::<Vec<_>>();
        if fields.len() != 5
            || !canonical_docker_id(fields[0])
            || fields[1] != self.owner
            || fields[2] != request.context.tenant_id.as_str()
            || fields[3] != request.sandbox_id.as_str()
        {
            return Err(SandboxError::Denied);
        }
        let running = match fields[4] {
            "true" => true,
            "false" => false,
            _ => return Err(SandboxError::OperationFailed),
        };
        Ok(Some(OwnedContainer {
            docker_id: fields[0].into(),
            running,
        }))
    }

    fn remove_owned(&self, request: &TargetRequest) -> Result<bool, SandboxError> {
        let Some(owned) = self
            .inspect_owned(request)
            .map_err(|_| SandboxError::OutcomeUnknown)?
        else {
            return Ok(false);
        };
        let mut args = self.base_args("rm");
        args.extend(["--force".into(), owned.docker_id]);
        let output = self.run_cli(&args, CONTROL_TIMEOUT)?;
        (output.code == 0)
            .then_some(true)
            .ok_or(SandboxError::OutcomeUnknown)
    }

    fn compensate_start(&self, request: &TargetRequest) -> Result<(), SandboxError> {
        self.remove_owned(request).map(|_| ())
    }

    fn run_cli(&self, arguments: &[String], timeout: Duration) -> Result<CliOutput, SandboxError> {
        let mut job = Exec::cmd(&self.executable)
            .args(arguments)
            .env_clear()
            .stdout(Redirection::Pipe)
            .stderr(Redirection::Pipe)
            .start()
            .map_err(|_| SandboxError::Unavailable)?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(SandboxError::InvalidRequest)?;
        let read = {
            let mut communicator = job
                .communicate()
                .map_err(|_| SandboxError::OperationFailed)?
                .limit_time(timeout)
                .limit_size(MAX_OUTPUT_BYTES + 1);
            communicator.read()
        };
        let (stdout, stderr) = match read {
            Ok(value) => value,
            Err(error) if error.kind() == ErrorKind::TimedOut => {
                let _ = job.kill();
                let _ = job.wait();
                return Err(SandboxError::Timeout);
            }
            Err(_) => {
                let _ = job.kill();
                let _ = job.wait();
                return Err(SandboxError::OperationFailed);
            }
        };
        if stdout.len().saturating_add(stderr.len()) > MAX_OUTPUT_BYTES {
            let _ = job.kill();
            let _ = job.wait();
            return Err(SandboxError::LimitExceeded);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(status) = job
            .wait_timeout(remaining)
            .map_err(|_| SandboxError::OperationFailed)?
        else {
            let _ = job.kill();
            let _ = job.wait();
            return Err(SandboxError::Timeout);
        };
        Ok(CliOutput {
            code: status
                .code()
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(128),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

impl Sandbox for DockerSandbox {
    fn start(&self, request: StartRequest) -> Result<StartResult, SandboxError> {
        let profile = self
            .profiles
            .get(&request.profile_id)
            .ok_or(SandboxError::InvalidRequest)?;
        let id = self.derived_id(&request)?;
        let target = TargetRequest {
            context: request.context.clone(),
            sandbox_id: id.clone(),
        };
        // Idempotent replay: a retry of an identical StartRequest derives the same
        // SandboxId. If a container already exists under that name, verify it is
        // owned by this tenant/owner before treating the retry as a safe no-op.
        // A name owned by a different tenant/owner is denied rather than reused or
        // silently overwritten by `docker run`.
        if let Some(owned) = self.inspect_owned(&target)? {
            return Ok(StartResult {
                sandbox_id: id,
                status: if owned.running {
                    SandboxStatus::Running
                } else {
                    SandboxStatus::Stopped
                },
            });
        }
        let mut args = self.base_args("run");
        args.extend([
            "--detach".into(),
            "--pull=never".into(),
            "--name".into(),
            Self::name(&id),
            "--label".into(),
            format!("{OWNER_LABEL}={}", self.owner),
            "--label".into(),
            format!("{TENANT_LABEL}={}", request.context.tenant_id.as_str()),
            "--label".into(),
            format!("{SANDBOX_LABEL}={}", id.as_str()),
            "--network".into(),
            "none".into(),
            "--read-only".into(),
            "--tmpfs".into(),
            "/workspace:rw,nosuid,nodev,size=268435456,mode=1777".into(),
            "--tmpfs".into(),
            "/tmp:rw,nosuid,nodev,noexec,size=67108864,mode=1777".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--user".into(),
            profile.user().into(),
            "--memory".into(),
            profile.memory_bytes().to_string(),
            "--cpus".into(),
            format!(
                "{}.{:03}",
                profile.cpu_millis() / 1000,
                profile.cpu_millis() % 1000
            ),
            "--pids-limit".into(),
            profile.pids().to_string(),
            profile.image().into(),
            "sleep".into(),
            "infinity".into(),
        ]);
        match self.run_cli(&args, CONTROL_TIMEOUT) {
            Ok(output) if output.code == 0 && canonical_docker_id(output.stdout.trim()) => {
                Ok(StartResult {
                    sandbox_id: id,
                    status: SandboxStatus::Running,
                })
            }
            Ok(_) | Err(_) => {
                // A `run` failure may be a benign name collision against a concurrent
                // identical request that already won the race (deterministic derived_id
                // means retries and races share the same container name). Re-inspect
                // before compensating: if this exact tenant/owner/sandbox already owns a
                // live container under this name, treat it as the idempotent-replay
                // success case rather than destroying it. A name owned by a different
                // tenant/owner is denied, never adopted or removed. Only a genuinely
                // absent container falls through to compensation.
                match self.inspect_owned(&target) {
                    Ok(Some(owned)) => Ok(StartResult {
                        sandbox_id: id,
                        status: if owned.running {
                            SandboxStatus::Running
                        } else {
                            SandboxStatus::Stopped
                        },
                    }),
                    Ok(None) => {
                        self.compensate_start(&target)
                            .map_err(|_| SandboxError::OutcomeUnknown)?;
                        Err(SandboxError::OperationFailed)
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult, SandboxError> {
        let target = TargetRequest {
            context: request.context.clone(),
            sandbox_id: request.sandbox_id.clone(),
        };
        let owned = self.inspect_owned(&target)?.ok_or(SandboxError::NotFound)?;
        if !owned.running {
            return Err(SandboxError::NotFound);
        }
        let mut args = self.base_args("exec");
        let workdir = if request.command.working_directory().is_empty() {
            "/workspace".to_owned()
        } else {
            format!("/workspace/{}", request.command.working_directory())
        };
        args.extend([
            "--workdir".into(),
            workdir,
            owned.docker_id,
            request.command.program().into(),
        ]);
        args.extend(request.command.arguments().iter().cloned());
        match self.run_cli(
            &args,
            Duration::from_millis(request.command.timeout_millis()),
        ) {
            Ok(output) => Ok(ExecuteResult {
                sandbox_id: request.sandbox_id,
                exit_code: output.code,
                stdout: output.stdout,
                stderr: output.stderr,
                truncated: false,
            }),
            Err(error @ (SandboxError::Timeout | SandboxError::LimitExceeded)) => {
                self.remove_owned(&target)
                    .map_err(|_| SandboxError::OutcomeUnknown)?;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    fn status(&self, request: TargetRequest) -> Result<StatusResult, SandboxError> {
        let owned = self
            .inspect_owned(&request)?
            .ok_or(SandboxError::NotFound)?;
        Ok(StatusResult {
            sandbox_id: request.sandbox_id,
            status: if owned.running {
                SandboxStatus::Running
            } else {
                SandboxStatus::Stopped
            },
        })
    }

    fn stop(&self, request: TargetRequest) -> Result<StopResult, SandboxError> {
        let id = request.sandbox_id.clone();
        let removed = self.remove_owned(&request)?;
        Ok(StopResult {
            sandbox_id: id,
            removed,
        })
    }
}

struct OwnedContainer {
    docker_id: String,
    running: bool,
}
struct CliOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Distinguishes a `docker inspect` failure that unambiguously means "no such
/// container" from any other provider failure (daemon unreachable, permission
/// denied, transient error). Only the former may be treated as `Ok(None)`.
fn confirmed_not_found(output: &CliOutput) -> bool {
    output.stderr.contains("No such container")
}

fn canonical_docker_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> (crate::ProfileId, DockerProfile) {
        let id = crate::ProfileId::new("rust").unwrap();
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

    #[test]
    fn constructor_accepts_only_absolute_cli_local_socket_owner_and_profiles() {
        let (id, profile) = profile();
        let profiles = BTreeMap::from([(id, profile)]);
        assert!(
            DockerSandbox::new(
                "/usr/bin/docker",
                "unix:///var/run/docker.sock",
                "a".repeat(64),
                profiles.clone(),
            )
            .is_ok()
        );
        for (cli, endpoint, owner) in [
            ("docker", "unix:///var/run/docker.sock", "a".repeat(64)),
            ("/usr/bin/docker", "tcp://localhost:2375", "a".repeat(64)),
            ("/usr/bin/docker", "unix://relative", "a".repeat(64)),
            (
                "/usr/bin/docker",
                "unix:///var/run/docker.sock",
                "bad".into(),
            ),
        ] {
            assert!(DockerSandbox::new(cli, endpoint, owner, profiles.clone()).is_err());
        }
    }

    #[test]
    fn sandbox_identity_is_deterministic_and_tenant_scoped() {
        let (id, profile) = profile();
        let adapter = DockerSandbox::new(
            "/usr/bin/docker",
            "unix:///var/run/docker.sock",
            "a".repeat(64),
            BTreeMap::from([(id.clone(), profile)]),
        )
        .unwrap();
        let request = |tenant: &str| StartRequest {
            context: crate::SandboxContext {
                tenant_id: crate::TenantId::new(tenant).unwrap(),
                principal_id: crate::PrincipalId::new("principal").unwrap(),
                request_id: crate::RequestId::new("request").unwrap(),
                correlation_id: crate::CorrelationId::new("correlation").unwrap(),
            },
            profile_id: id.clone(),
        };
        assert_eq!(
            adapter.derived_id(&request("tenant")).unwrap(),
            adapter.derived_id(&request("tenant")).unwrap()
        );
        assert_ne!(
            adapter.derived_id(&request("tenant")).unwrap(),
            adapter.derived_id(&request("other")).unwrap()
        );
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    #[ignore = "opt-in: requires RUST_FACTORY_SANDBOX_IMAGE pre-pulled by digest"]
    fn real_rust_container_start_execute_status_stop() {
        let image = std::env::var("RUST_FACTORY_SANDBOX_IMAGE").expect("immutable image");
        let profile_id = crate::ProfileId::new("rust").unwrap();
        let profile = DockerProfile::new(image, "1000", 512 * 1024 * 1024, 1000, 64).unwrap();
        let adapter = DockerSandbox::new(
            "/usr/local/bin/docker",
            "unix:///var/run/docker.sock",
            format!(
                "{:x}",
                Sha256::digest(format!("sandbox-test-{}", std::process::id()))
            ),
            BTreeMap::from([(profile_id.clone(), profile)]),
        )
        .unwrap();
        let suffix = std::process::id().to_string();
        let context = crate::SandboxContext {
            tenant_id: crate::TenantId::new("test").unwrap(),
            principal_id: crate::PrincipalId::new("tester").unwrap(),
            request_id: crate::RequestId::new(format!("request-{suffix}")).unwrap(),
            correlation_id: crate::CorrelationId::new(format!("correlation-{suffix}")).unwrap(),
        };
        let started = adapter.start(StartRequest {
            context: context.clone(),
            profile_id,
        });
        let (executed, inspected, stopped) = match started {
            Ok(started) => {
                let target = TargetRequest {
                    context: context.clone(),
                    sandbox_id: started.sandbox_id.clone(),
                };
                let executed = adapter.execute(ExecuteRequest {
                    context: context.clone(),
                    sandbox_id: started.sandbox_id,
                    command: crate::Command::new("cargo", vec!["--version".into()], "", 30_000)
                        .unwrap(),
                });
                let inspected = adapter.status(target.clone());
                let stopped = adapter.stop(target);
                (executed, inspected, stopped)
            }
            Err(error) => panic!("start failed safely: {error}"),
        };
        assert_eq!(executed.unwrap().exit_code, 0);
        assert_eq!(inspected.unwrap().status, SandboxStatus::Running);
        assert!(stopped.unwrap().removed);
    }
}
