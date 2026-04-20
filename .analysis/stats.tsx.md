# Analysis: context/stats.tsx

**Summary**: Generic stats store with counter, gauge, timer (histogram), and set metrics. React context provider + hooks for consuming stats.

## Dependencies

- `react` → Context API (createContext, useContext, useEffect, useMemo, useCallback)
- `../utils/config.js` → saveCurrentProjectConfig (persistence on exit)

## Structure

### Core Types

- **StatsStore** interface: `{ increment(name, value?), set(name, value), observe(name, value), add(name, value: string), getAll() }`
- **Histogram** — internal: `{ reservoir: number[], count, sum, min, max }`
- Uses reservoir sampling (Algorithm R) with RESERVOIR_SIZE = 1024

### Functions

1. **createStatsStore()** — Factory: returns StatsStore with internal Maps for metrics, histograms, sets.
2. **increment(name, value=1)** — Counter: add to metric value.
3. **set(name, value)** — Gauge: replace metric value.
4. **observe(name, value)** — Timer: add sample to histogram (reservoir + aggregate stats).
5. **add(name, value: string)** — Set: add string to dedup set (counts cardinality).
6. **getAll()** — Serialize all metrics: basic counters + histogram aggregations (count, min, max, avg, p50, p95, p99) + set cardinality.
7. **percentile(sorted, p)** — Linear interpolation percentile from sorted samples.
8. **StatsProvider** component — React context setup with internal store creation, exit flush handler.
9. **useStats()** — Hook to access store; throws if not in provider.
10. **useCounter, useGauge, useTimer, useSet** — Specialized hooks returning bound functions.

## Issues（Rust Porting Concerns）

- [ ] ISSUE [MEDIUM]: Histogram percentile calculation assumes sorted array is stable. If histogram is mutated during sorting (race condition), results are wrong.
  Impact: In concurrent Rust code, must use Arc<Mutex<>> or atomic operations.
  Suggestion: Use `parking_lot::Mutex` for lower latency; ensure all accesses lock consistently.

- [ ] ISSUE [MEDIUM]: Reservoir sampling modifies internal state (h.reservoir, h.count, h.sum) without synchronization. In multi-threaded Rust, this is undefined behavior.
  Impact: If multiple threads call observe() concurrently, histogram corruption.
  Suggestion: Wrap histogram in Mutex; use Arc for shared access.

- [ ] ISSUE [MEDIUM]: `Math.random()` in Algorithm R is not cryptographically secure but acceptable for sampling. In Rust, use `rand` crate with seeded RNG for reproducibility.
  Impact: Percentile estimates are not deterministic.
  Suggestion: Use `rand::thread_rng()` or seed with session ID for reproducible results.

- [ ] ISSUE [LOW]: getAll() rebuilds result object every call (O(n) iteration). No caching.
  Impact: If called frequently, wasteful.
  Suggestion: Cache result; invalidate on mutation.

- [ ] ISSUE [LOW]: Set cardinality (add/getAll) uses JS Set which counts unique strings. In Rust, use HashSet<String>.
  Impact: No functional impact, just type difference.
  Suggestion: Use `HashSet<String>` in Rust version.

- [ ] ISSUE [LOW]: No validation of observation values (observe() accepts negative numbers, infinity, NaN). Results in corrupted histogram stats.
  Impact: Stats become meaningless if bad values sneak in.
  Suggestion: Add `assert!( value.is_finite() )` or use input validation.

## Optimizations

- [ ] OPT [PERF]: Reservoir sampling Algorithm R is O(1) space for unbounded stream, but here size is fixed (1024 samples). For Sage's use case (typically < 100 samples), could use simpler Vitter's algorithm or just keep all samples.
  Why better: Simpler logic, exact percentiles instead of estimates.
  Approach: If samples < 10k, store all; only use reservoir for larger streams.

- [ ] OPT [SAFETY]: Percentile calculation with linear interpolation assumes sorted array has at least 1 element. If count=0, accessing sorted[lower] panics.
  Impact: getAll() crashes if histogram observed but count=0.
  Suggestion: Add guard: if count=0, return empty result or skip histogram aggregation.

- [ ] OPT [IDIOM]: React hooks pattern (useCounter, useGauge, useTimer, useSet) is idiomatic but repetitive. Could use generic useMetric(name, op) hook.
  Why better: DRY principle.
  Approach: `useMetric('requests', 'increment')` instead of separate hooks.

- [ ] OPT [ERGONOMICS]: getAll() returns flat Record<string, number> with dynamic keys. Caller must know key names (e.g., "timer_p95"). Consider returning typed struct.
  Why better: Type safety, IDE autocomplete.
  Approach: Define interface for aggregated stats; export getAll() with correct return type.

## Sage Porting

Rust equivalent:
```rust
use std::collections::{HashMap, HashSet};
use parking_lot::Mutex;

pub struct Histogram {
  reservoir: Vec<f64>,
  count: u64,
  sum: f64,
  min: f64,
  max: f64,
}

pub struct StatsStore {
  metrics: Mutex<HashMap<String, f64>>,
  histograms: Mutex<HashMap<String, Histogram>>,
  sets: Mutex<HashMap<String, HashSet<String>>>,
}

impl StatsStore {
  pub fn observe(&self, name: &str, value: f64) {
    assert!(value.is_finite());
    let mut h_map = self.histograms.lock();
    let h = h_map.entry(name.to_string())
      .or_insert_with(|| Histogram { ... });
    // Algorithm R reservoir sampling
  }

  pub fn get_all(&self) -> HashMap<String, f64> {
    // Aggregate + calculate percentiles
  }
}
```

**React context → Rust pattern**:
- No context API in Rust; use Arc<Mutex<StatsStore>> passed through function parameters or thread-local storage.
- Exit handler: register via `atexit` crate or signal handler (`signal_hook`).

## Type Mapping

**StatsStore interface** → Rust trait/struct:
```rust
pub trait StatsStore {
  fn increment(&mut self, name: &str, value: f64);
  fn set(&mut self, name: &str, value: f64);
  fn observe(&mut self, name: &str, value: f64);
  fn add(&mut self, name: &str, value: String);
  fn get_all(&self) -> HashMap<String, f64>;
}
```

**React Provider pattern** → Rust dependency injection:
- No React; instead, create StatsStore at app startup + pass Arc<Mutex<StatsStore>> to all components.
