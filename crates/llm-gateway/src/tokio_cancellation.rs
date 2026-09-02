//! [`CancellationSignalFactory`]/[`CancellationHandle`] backed by
//! `tokio_util::sync::CancellationToken`.
//!
//! Replaces a previously hand-rolled `Waker`-registry-based cancellation
//! primitive: `CancellationToken` already provides the same broadcast,
//! no-lost-wake guarantee, battle-tested at far larger scale than this
//! crate's own bespoke implementation ever was. This module owns no
//! Tokio runtime; it only wraps a runtime-neutral value type
//! (`CancellationToken` itself does not require an active `tokio` runtime
//! to construct, clone, or cancel — only `.cancelled().await` needs a
//! runtime to poll).

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::{
    CancellationFuture, CancellationHandle, CancellationSignal, CancellationSignalFactory,
};

/// A [`CancellationHandle`] backed by one `CancellationToken`.
#[derive(Clone, Debug, Default)]
pub struct TokioCancellationHandle(CancellationToken);

impl CancellationSignal for TokioCancellationHandle {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    fn cancelled(&self) -> CancellationFuture<'_> {
        Box::pin(self.0.cancelled())
    }
}

impl CancellationHandle for TokioCancellationHandle {
    fn cancel(&self) {
        self.0.cancel();
    }
}

/// Creates a fresh, independent [`TokioCancellationHandle`] for each call:
/// each returned handle has its own `CancellationToken`, not a shared
/// child of some prior token.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokioCancellationSignalFactory;

impl CancellationSignalFactory for TokioCancellationSignalFactory {
    fn create(&self) -> Arc<dyn CancellationHandle> {
        Arc::new(TokioCancellationHandle::default())
    }
}

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use super::*;

    #[test]
    fn a_fresh_handle_is_not_cancelled() {
        let handle = TokioCancellationSignalFactory.create();
        assert!(!handle.is_cancelled());
    }

    #[test]
    fn cancel_is_observable_through_is_cancelled() {
        let handle = TokioCancellationSignalFactory.create();
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let handle = TokioCancellationSignalFactory.create();
        handle.cancel();
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn two_handles_from_the_same_factory_are_independent() {
        let factory = TokioCancellationSignalFactory;
        let first = factory.create();
        let second = factory.create();
        first.cancel();
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
    }

    #[test]
    fn cancel_before_wait_completes_immediately() {
        let handle = TokioCancellationSignalFactory.create();
        handle.cancel();
        let mut future = handle.cancelled();
        let waker = futures_test_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert_eq!(
            std::future::Future::poll(future.as_mut(), &mut context),
            Poll::Ready(())
        );
    }

    #[test]
    fn a_pending_wait_wakes_when_cancel_is_called() {
        // A real .await requires a Tokio runtime to poll the returned
        // future to completion; this crate's core stays runtime-neutral,
        // so this test drives the future manually with a no-op waker and
        // asserts it stays Pending until cancel() is called, then becomes
        // Ready on the next poll (CancellationToken's own wake mechanics
        // are third-party-tested and are not re-verified here).
        let handle = TokioCancellationSignalFactory.create();
        let mut future = handle.cancelled();
        let waker = futures_test_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert_eq!(
            std::future::Future::poll(future.as_mut(), &mut context),
            Poll::Pending
        );
        handle.cancel();
        assert_eq!(
            std::future::Future::poll(future.as_mut(), &mut context),
            Poll::Ready(())
        );
    }

    fn futures_test_waker() -> std::task::Waker {
        std::task::Waker::noop().clone()
    }
}
