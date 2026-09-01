use crate::{SandboxError, validation};

pub const MAX_ID_BYTES: usize = 128;
pub const MAX_IMAGE_BYTES: usize = 512;
pub const MAX_PROGRAM_BYTES: usize = 256;
pub const MAX_ARGUMENTS: usize = 32;
pub const MAX_ARGUMENT_BYTES: usize = 4_096;
pub const MAX_ARGUMENTS_BYTES: usize = 16 * 1_024;
pub const MAX_WORKING_DIRECTORY_BYTES: usize = 1_024;
pub const MAX_OUTPUT_BYTES: usize = 32 * 1_024;
pub const MAX_TIMEOUT_MILLIS: u64 = 5 * 60 * 1_000;

macro_rules! logical_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, SandboxError> {
                let value = value.into();
                validation::logical_id(&value)
                    .then_some(Self(value))
                    .ok_or(SandboxError::InvalidRequest)
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
logical_id!(TenantId);
logical_id!(PrincipalId);
logical_id!(RequestId);
logical_id!(CorrelationId);
logical_id!(ProfileId);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SandboxId(String);
impl SandboxId {
    pub fn new(value: impl Into<String>) -> Result<Self, SandboxError> {
        let value = value.into();
        validation::sandbox_id(&value)
            .then_some(Self(value))
            .ok_or(SandboxError::InvalidRequest)
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxContext {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub request_id: RequestId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    program: String,
    arguments: Vec<String>,
    working_directory: String,
    timeout_millis: u64,
}
impl Command {
    pub fn new(
        program: impl Into<String>,
        arguments: Vec<String>,
        working_directory: impl Into<String>,
        timeout_millis: u64,
    ) -> Result<Self, SandboxError> {
        let program = program.into();
        let working_directory = working_directory.into();
        let argument_bytes = arguments.iter().try_fold(0usize, |total, value| {
            total
                .checked_add(value.len())
                .ok_or(SandboxError::LimitExceeded)
        })?;
        if program.len() > MAX_PROGRAM_BYTES
            || arguments.len() > MAX_ARGUMENTS
            || arguments
                .iter()
                .any(|value| value.len() > MAX_ARGUMENT_BYTES)
            || argument_bytes > MAX_ARGUMENTS_BYTES
            || working_directory.len() > MAX_WORKING_DIRECTORY_BYTES
            || timeout_millis > MAX_TIMEOUT_MILLIS
        {
            return Err(SandboxError::LimitExceeded);
        }
        let valid = validation::bounded_text(&program, MAX_PROGRAM_BYTES, false)
            && arguments
                .iter()
                .all(|value| validation::bounded_text(value, MAX_ARGUMENT_BYTES, true))
            && validation::relative_path(&working_directory)
            && timeout_millis > 0;
        valid
            .then_some(Self {
                program,
                arguments,
                working_directory,
                timeout_millis,
            })
            .ok_or(SandboxError::InvalidRequest)
    }
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
    #[must_use]
    pub fn working_directory(&self) -> &str {
        &self.working_directory
    }
    #[must_use]
    pub const fn timeout_millis(&self) -> u64 {
        self.timeout_millis
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartRequest {
    pub context: SandboxContext,
    pub profile_id: ProfileId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecuteRequest {
    pub context: SandboxContext,
    pub sandbox_id: SandboxId,
    pub command: Command,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetRequest {
    pub context: SandboxContext,
    pub sandbox_id: SandboxId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxStatus {
    Running,
    Stopped,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartResult {
    pub sandbox_id: SandboxId,
    pub status: SandboxStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecuteResult {
    pub sandbox_id: SandboxId,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusResult {
    pub sandbox_id: SandboxId,
    pub status: SandboxStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopResult {
    pub sandbox_id: SandboxId,
    pub removed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxOperation {
    Start,
    Execute,
    Status,
    Stop,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxEvent {
    pub operation: SandboxOperation,
    pub sandbox_id: Option<SandboxId>,
    pub status: Option<SandboxStatus>,
    pub tenant_id: TenantId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerProfile {
    image: String,
    user: String,
    memory_bytes: u64,
    cpu_millis: u32,
    pids: u32,
}
impl DockerProfile {
    pub fn new(
        image: impl Into<String>,
        user: impl Into<String>,
        memory_bytes: u64,
        cpu_millis: u32,
        pids: u32,
    ) -> Result<Self, SandboxError> {
        let image = image.into();
        let user = user.into();
        let numeric_non_root =
            user.parse::<u32>().is_ok_and(|value| value > 0) && !user.starts_with('0');
        (validation::immutable_image(&image)
            && numeric_non_root
            && (64 * 1024 * 1024..=2 * 1024 * 1024 * 1024).contains(&memory_bytes)
            && (100..=4_000).contains(&cpu_millis)
            && (16..=256).contains(&pids))
        .then_some(Self {
            image,
            user,
            memory_bytes,
            cpu_millis,
            pids,
        })
        .ok_or(SandboxError::InvalidRequest)
    }
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }
    #[must_use]
    pub const fn cpu_millis(&self) -> u32 {
        self.cpu_millis
    }
    #[must_use]
    pub const fn pids(&self) -> u32 {
        self.pids
    }
}
