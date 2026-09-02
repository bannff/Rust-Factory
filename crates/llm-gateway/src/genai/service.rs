//! Shared caller-polled invocation orchestration.

use std::future::Future;

pub(crate) enum InvocationOutcome<T> {
    Provider(T),
    Cancelled,
    DeadlineExceeded,
}

/// `biased;` forces top-to-bottom evaluation in written order and stops at
/// the first branch that is `Ready`, matching the fixed-priority
/// cancellation -> deadline -> provider cascade this replaced (a hand-rolled
/// `poll_fn` with the identical short-circuit order). Without `biased;`,
/// `tokio::select!` checks branches in a random order for fairness, which
/// would silently drop cancellation's priority over an already-ready
/// provider result.
///
/// Once one branch resolves, `select!` drops every other branch's future
/// (same as the replaced `poll_fn` closure's pinned futures being dropped
/// when the outer future resolves). In particular, a `Cancelled` or
/// `DeadlineExceeded` outcome drops the in-flight `provider` future: the
/// caller relies on this to actually stop the underlying request, not just
/// to stop *awaiting* it.
pub(crate) async fn race_invocation<P, C, D, T>(
    provider: P,
    cancellation: C,
    deadline: D,
) -> InvocationOutcome<T>
where
    P: Future<Output = T>,
    C: Future<Output = ()>,
    D: Future<Output = ()>,
{
    tokio::select! {
        biased;
        () = cancellation => InvocationOutcome::Cancelled,
        () = deadline => InvocationOutcome::DeadlineExceeded,
        result = provider => InvocationOutcome::Provider(result),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll},
    };

    use super::*;

    struct Counted<T> {
        polls: Arc<AtomicUsize>,
        output: Option<T>,
    }

    impl<T: Unpin> Future for Counted<T> {
        type Output = T;

        fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<T> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.output.take().map_or(Poll::Pending, Poll::Ready)
        }
    }

    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(std::task::Waker::noop()))
    }

    fn counted<T>(output: Option<T>) -> (Counted<T>, Arc<AtomicUsize>) {
        let polls = Arc::new(AtomicUsize::new(0));
        (
            Counted {
                polls: Arc::clone(&polls),
                output,
            },
            polls,
        )
    }

    #[test]
    fn cancellation_wins_when_every_future_is_ready() {
        let (provider, provider_polls) = counted(Some("provider"));
        let (cancellation, cancellation_polls) = counted(Some(()));
        let (deadline, deadline_polls) = counted(Some(()));
        let mut race = std::pin::pin!(race_invocation(provider, cancellation, deadline));

        assert!(matches!(
            poll_once(race.as_mut()),
            Poll::Ready(InvocationOutcome::Cancelled)
        ));
        assert_eq!(cancellation_polls.load(Ordering::SeqCst), 1);
        assert_eq!(deadline_polls.load(Ordering::SeqCst), 0);
        assert_eq!(provider_polls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn deadline_wins_over_provider_when_cancellation_is_pending() {
        let (provider, provider_polls) = counted(Some("provider"));
        let (cancellation, cancellation_polls) = counted(None::<()>);
        let (deadline, deadline_polls) = counted(Some(()));
        let mut race = std::pin::pin!(race_invocation(provider, cancellation, deadline));

        assert!(matches!(
            poll_once(race.as_mut()),
            Poll::Ready(InvocationOutcome::DeadlineExceeded)
        ));
        assert_eq!(cancellation_polls.load(Ordering::SeqCst), 1);
        assert_eq!(deadline_polls.load(Ordering::SeqCst), 1);
        assert_eq!(provider_polls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn provider_completes_after_both_controls_are_checked() {
        let (provider, provider_polls) = counted(Some("provider"));
        let (cancellation, cancellation_polls) = counted(None::<()>);
        let (deadline, deadline_polls) = counted(None::<()>);
        let mut race = std::pin::pin!(race_invocation(provider, cancellation, deadline));

        assert!(matches!(
            poll_once(race.as_mut()),
            Poll::Ready(InvocationOutcome::Provider("provider"))
        ));
        assert_eq!(cancellation_polls.load(Ordering::SeqCst), 1);
        assert_eq!(deadline_polls.load(Ordering::SeqCst), 1);
        assert_eq!(provider_polls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn no_branch_short_circuits_when_every_future_is_pending() {
        let (provider, provider_polls) = counted(None::<()>);
        let (cancellation, cancellation_polls) = counted(None::<()>);
        let (deadline, deadline_polls) = counted(None::<()>);
        let mut race = std::pin::pin!(race_invocation(provider, cancellation, deadline));

        assert!(poll_once(race.as_mut()).is_pending());
        assert_eq!(cancellation_polls.load(Ordering::SeqCst), 1);
        assert_eq!(deadline_polls.load(Ordering::SeqCst), 1);
        assert_eq!(provider_polls.load(Ordering::SeqCst), 1);
    }
}
