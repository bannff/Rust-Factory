//! Runtime-neutral asynchronous provider ports.

use std::{future::Future, pin::Pin, sync::Arc, time::Instant};

use crate::{GenerateRequest, GenerateResponse, IdempotencyKey, LlmError};

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<GenerateResponse, LlmError>> + Send + 'a>>;
pub type CancellationFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
pub type DeadlineFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub trait CancellationSignal: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn cancelled(&self) -> CancellationFuture<'_>;
}

/// A [`CancellationSignal`] that can also be triggered. Read-only consumers
/// (provider adapters, [`InvocationControl`]) hold `&dyn CancellationSignal`;
/// the producer that owns the invocation's lifecycle (e.g. a workflow runner
/// handling an external cancel request) holds `Arc<dyn CancellationHandle>`.
pub trait CancellationHandle: CancellationSignal {
    fn cancel(&self);
}

/// Creates a fresh, unparameterized, not-yet-cancelled [`CancellationHandle`]
/// for one invocation. Unlike [`DeadlineFactory::create`], this takes no
/// parameter: every fresh cancellation starts in the same state.
pub trait CancellationSignalFactory: Send + Sync {
    fn create(&self) -> Arc<dyn CancellationHandle>;
}

pub trait DeadlineSignal: Send + Sync {
    fn instant(&self) -> Instant;
    fn is_elapsed(&self) -> bool;
    fn elapsed(&self) -> DeadlineFuture<'_>;
}

pub trait DeadlineFactory: Send + Sync {
    fn create(&self, instant: Instant) -> Box<dyn DeadlineSignal>;
}

#[derive(Clone, Copy)]
pub struct InvocationControl<'a> {
    pub idempotency_key: &'a IdempotencyKey,
    pub cancellation: &'a dyn CancellationSignal,
    pub deadline: &'a dyn DeadlineSignal,
}

impl InvocationControl<'_> {
    pub fn preflight(&self) -> Result<(), LlmError> {
        if self.cancellation.is_cancelled() {
            Err(LlmError::Cancelled)
        } else if self.deadline.is_elapsed() {
            Err(LlmError::DeadlineExceeded)
        } else {
            Ok(())
        }
    }
}

pub trait LlmProvider: Send + Sync {
    fn generate<'a>(
        &'a self,
        request: &'a GenerateRequest,
        control: InvocationControl<'a>,
    ) -> ProviderFuture<'a>;
}
