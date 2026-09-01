use crate::{
    EventSubmission, ExecuteRequest, ExecuteResult, Sandbox, SandboxError, SandboxEvent,
    SandboxEventSink, SandboxOperation, StartRequest, StartResult, StatusResult, StopResult,
    TargetRequest,
};

/// Thin validation and telemetry wrapper around one concrete Sandbox adapter.
pub struct SandboxService<S, E> {
    sandbox: S,
    events: E,
}
impl<S, E> SandboxService<S, E> {
    #[must_use]
    pub const fn new(sandbox: S, events: E) -> Self {
        Self { sandbox, events }
    }
}
impl<S: Sandbox, E: SandboxEventSink> SandboxService<S, E> {
    fn emit(
        &self,
        operation: SandboxOperation,
        context: &crate::SandboxContext,
        id: Option<crate::SandboxId>,
        status: Option<crate::SandboxStatus>,
    ) {
        let _ = self.events.try_emit(SandboxEvent {
            operation,
            sandbox_id: id,
            status,
            correlation_id: context.correlation_id.clone(),
        });
    }
}
impl<S: Sandbox, E: SandboxEventSink> Sandbox for SandboxService<S, E> {
    fn start(&self, request: StartRequest) -> Result<StartResult, SandboxError> {
        let context = request.context.clone();
        let result = self.sandbox.start(request)?;
        self.emit(
            SandboxOperation::Start,
            &context,
            Some(result.sandbox_id.clone()),
            Some(result.status),
        );
        Ok(result)
    }
    fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult, SandboxError> {
        let context = request.context.clone();
        let id = request.sandbox_id.clone();
        let result = self.sandbox.execute(request)?;
        self.emit(SandboxOperation::Execute, &context, Some(id), None);
        Ok(result)
    }
    fn status(&self, request: TargetRequest) -> Result<StatusResult, SandboxError> {
        let context = request.context.clone();
        let result = self.sandbox.status(request)?;
        self.emit(
            SandboxOperation::Status,
            &context,
            Some(result.sandbox_id.clone()),
            Some(result.status),
        );
        Ok(result)
    }
    fn stop(&self, request: TargetRequest) -> Result<StopResult, SandboxError> {
        let context = request.context.clone();
        let id = request.sandbox_id.clone();
        let result = self.sandbox.stop(request)?;
        self.emit(
            SandboxOperation::Stop,
            &context,
            Some(id),
            Some(crate::SandboxStatus::Stopped),
        );
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DropEvents;
impl SandboxEventSink for DropEvents {
    fn try_emit(&self, _: SandboxEvent) -> EventSubmission {
        EventSubmission::Dropped
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DenySandbox;
impl Sandbox for DenySandbox {
    fn start(&self, _: StartRequest) -> Result<StartResult, SandboxError> {
        Err(SandboxError::Denied)
    }
    fn execute(&self, _: ExecuteRequest) -> Result<ExecuteResult, SandboxError> {
        Err(SandboxError::Denied)
    }
    fn status(&self, _: TargetRequest) -> Result<StatusResult, SandboxError> {
        Err(SandboxError::Denied)
    }
    fn stop(&self, _: TargetRequest) -> Result<StopResult, SandboxError> {
        Err(SandboxError::Denied)
    }
}
