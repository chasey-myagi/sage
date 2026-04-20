//! Sleep helper that respects cancellation.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/sleep.ts`.

use std::time::Duration;

use tokio::time;
use tokio_util::sync::CancellationToken;

/// Sleep for `ms` milliseconds.
pub async fn sleep(ms: u64) {
    time::sleep(Duration::from_millis(ms)).await;
}

/// Sleep for `ms` milliseconds, returning early if the token is cancelled.
/// Returns `true` if sleep completed, `false` if cancelled.
pub async fn sleep_cancellable(ms: u64, cancel: CancellationToken) -> bool {
    tokio::select! {
        _ = time::sleep(Duration::from_millis(ms)) => true,
        _ = cancel.cancelled() => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn sleep_completes() {
        sleep(1).await;
    }

    #[tokio::test]
    async fn sleep_cancellable_completes() {
        let token = CancellationToken::new();
        let completed = sleep_cancellable(1, token).await;
        assert!(completed);
    }

    #[tokio::test]
    async fn sleep_cancellable_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        let completed = sleep_cancellable(10_000, token).await;
        assert!(!completed);
    }
}
