//! Central timing instrumentation for startup profiling.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/timings.ts`.
//!
//! Enable with `PI_TIMING=1` environment variable.

use std::sync::Mutex;
use std::time::Instant;

struct TimingEntry {
    label: String,
    ms: u128,
}

struct TimingsState {
    entries: Vec<TimingEntry>,
    last: Instant,
}

static STATE: Mutex<Option<TimingsState>> = Mutex::new(None);

fn is_enabled() -> bool {
    std::env::var("PI_TIMING").as_deref() == Ok("1")
}

/// Reset the timing accumulator and start fresh.
///
/// Mirrors `resetTimings()` from TypeScript.
pub fn reset_timings() {
    if !is_enabled() {
        return;
    }
    let mut guard = STATE.lock().unwrap();
    *guard = Some(TimingsState {
        entries: Vec::new(),
        last: Instant::now(),
    });
}

/// Record elapsed time since the last `time()` call (or `reset_timings()`).
///
/// Mirrors `time()` from TypeScript.
pub fn time(label: &str) {
    if !is_enabled() {
        return;
    }
    let mut guard = STATE.lock().unwrap();
    if let Some(ref mut state) = *guard {
        let now = Instant::now();
        let ms = now.duration_since(state.last).as_millis();
        state.entries.push(TimingEntry {
            label: label.to_string(),
            ms,
        });
        state.last = now;
    } else {
        // Auto-initialize
        drop(guard);
        reset_timings();
        time(label);
    }
}

/// Print all recorded timings to stderr.
///
/// Mirrors `printTimings()` from TypeScript.
pub fn print_timings() {
    if !is_enabled() {
        return;
    }
    let guard = STATE.lock().unwrap();
    if let Some(ref state) = *guard {
        if state.entries.is_empty() {
            return;
        }
        eprintln!("\n--- Startup Timings ---");
        let total: u128 = state.entries.iter().map(|e| e.ms).sum();
        for entry in &state.entries {
            eprintln!("  {}: {}ms", entry.label, entry.ms);
        }
        eprintln!("  TOTAL: {total}ms");
        eprintln!("------------------------\n");
    }
}

#[cfg(test)]
mod tests {
    // Timings tests are environment-dependent; just verify the API compiles.
    use super::*;

    #[test]
    fn api_compiles() {
        reset_timings();
        time("step1");
        print_timings();
    }
}
