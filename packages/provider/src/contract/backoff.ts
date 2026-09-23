/**
 * Abortable wait and the exponential backoff curve shared by both retry
 * layers: provider pre-stream retries and core whole-turn retries.
 *
 * Each layer owns its own base delay and attempt limit; only the curve and
 * the timer live here. No I/O: the only side effect is a timer.
 *
 * Intentionally not in the Log: waits and computed delays. Callers record the
 * retry itself (usage details, `turn_retry`), never the sleep.
 */

/** Waits `ms` milliseconds; resolves early, never rejects, when `signal` aborts. */
export type Sleeper = (ms: number, signal?: AbortSignal) => Promise<void>;

/** Growth factor between consecutive retry delays. */
const BACKOFF_FACTOR = 4;

/**
 * Delay before retry number `attempt` (1-based): `baseMs`, then
 * `4 × baseMs`, then `16 × baseMs`, and so on.
 */
export function backoffMs(attempt: number, baseMs: number): number {
  return baseMs * BACKOFF_FACTOR ** (attempt - 1);
}

/**
 * Real-timer `Sleeper`. Resolves immediately when `signal` is already
 * aborted, and resolves (does not reject) as soon as it aborts mid-wait,
 * clearing the timer and the listener. Callers check `signal.aborted`
 * afterwards to tell an abort from a completed wait.
 */
export const sleep: Sleeper = (ms, signal) =>
  new Promise((resolve) => {
    if (signal?.aborted) {
      resolve();
      return;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    const finish = (): void => {
      if (timer !== undefined) clearTimeout(timer);
      signal?.removeEventListener("abort", finish);
      resolve();
    };
    timer = setTimeout(finish, ms);
    signal?.addEventListener("abort", finish, { once: true });
  });
