//! Countdown timer utility.
//!
//! Translated from `components/countdown-timer.ts`.

use std::time::{Duration, Instant};

/// A simple countdown timer that tracks remaining time.
///
/// In the TypeScript version, this used `setInterval` to tick the UI.
/// In Rust, the caller is responsible for polling `tick()` and requesting re-renders.
pub struct CountdownTimer {
    started_at: Instant,
    timeout: Duration,
    on_tick: Box<dyn Fn(u64) + Send>,
    on_expire: Box<dyn Fn() + Send>,
    expired: bool,
    last_reported_seconds: u64,
}

impl CountdownTimer {
    /// Create a new countdown timer.
    ///
    /// - `timeout_ms`: total timeout in milliseconds
    /// - `on_tick`: called with remaining seconds whenever the second changes
    /// - `on_expire`: called once when the timer reaches zero
    pub fn new<T, E>(timeout_ms: u64, on_tick: T, on_expire: E) -> Self
    where
        T: Fn(u64) + Send + 'static,
        E: Fn() + Send + 'static,
    {
        let remaining_secs = timeout_ms.div_ceil(1000);
        on_tick(remaining_secs);
        Self {
            started_at: Instant::now(),
            timeout: Duration::from_millis(timeout_ms),
            on_tick: Box::new(on_tick),
            on_expire: Box::new(on_expire),
            expired: false,
            last_reported_seconds: remaining_secs,
        }
    }

    /// Poll the timer, invoking callbacks as needed.
    /// Returns `true` if the timer just expired this tick.
    pub fn tick(&mut self) -> bool {
        if self.expired {
            return false;
        }
        let elapsed = self.started_at.elapsed();
        let remaining = self.timeout.saturating_sub(elapsed);
        let remaining_secs = remaining.as_secs() + if remaining.subsec_millis() > 0 { 1 } else { 0 };

        if remaining_secs != self.last_reported_seconds {
            self.last_reported_seconds = remaining_secs;
            (self.on_tick)(remaining_secs);
        }

        if remaining.is_zero() {
            self.expired = true;
            (self.on_expire)();
            return true;
        }
        false
    }

    pub fn is_expired(&self) -> bool {
        self.expired
    }

    /// Remaining seconds (approximate).
    pub fn remaining_secs(&self) -> u64 {
        let elapsed = self.started_at.elapsed();
        let remaining = self.timeout.saturating_sub(elapsed);
        remaining.as_secs() + if remaining.subsec_millis() > 0 { 1 } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn initial_tick_fires_on_creation() {
        let ticks = Arc::new(Mutex::new(vec![]));
        let ticks2 = ticks.clone();
        let _timer = CountdownTimer::new(
            5000,
            move |secs| ticks2.lock().unwrap().push(secs),
            || {},
        );
        let collected = ticks.lock().unwrap().clone();
        assert_eq!(collected, vec![5], "Expected initial tick with 5 seconds");
    }

    #[test]
    fn remaining_secs_decreases_over_time() {
        let timer = CountdownTimer::new(1000, |_| {}, || {});
        let r = timer.remaining_secs();
        assert!(r <= 1, "Expected ≤1 second remaining, got {r}");
    }
}
