use std::sync::Arc;

use crate::{
    ExecuteRequest, ExecuteResult, SandboxError, SandboxEvent, StartRequest, StartResult,
    StatusResult, StopResult, TargetRequest,
};

pub trait Sandbox: Send + Sync {
    fn start(&self, request: StartRequest) -> Result<StartResult, SandboxError>;
    fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult, SandboxError>;
    fn status(&self, request: TargetRequest) -> Result<StatusResult, SandboxError>;
    fn stop(&self, request: TargetRequest) -> Result<StopResult, SandboxError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSubmission {
    Accepted,
    Dropped,
}

/// Best-effort and non-blocking. Implementations must not perform network or disk I/O.
pub trait SandboxEventSink: Send + Sync {
    fn try_emit(&self, event: SandboxEvent) -> EventSubmission;
}

impl<T: Sandbox + ?Sized> Sandbox for Arc<T> {
    fn start(&self, request: StartRequest) -> Result<StartResult, SandboxError> {
        (**self).start(request)
    }
    fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult, SandboxError> {
        (**self).execute(request)
    }
    fn status(&self, request: TargetRequest) -> Result<StatusResult, SandboxError> {
        (**self).status(request)
    }
    fn stop(&self, request: TargetRequest) -> Result<StopResult, SandboxError> {
        (**self).stop(request)
    }
}
