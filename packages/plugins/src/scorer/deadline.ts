/**
 * Deadline for one scorer call: a timer and the caller's signal folded into
 * one abort signal.
 *
 * Contract: `signal` aborts when `timeoutMs` elapses or the caller's signal
 * aborts, whichever comes first, and `cause()` names that first source. A
 * caller signal that is already aborted makes the deadline start aborted.
 *
 * Intentionally not in the Log: the deadline. Scorers report its outcome
 * as a `timeout` or `aborted` fallback.
 */

/** Largest delay a Node timer accepts; longer or non-finite timeouts are clamped to it. */
const MAX_TIMER_MS = 2_147_483_647;

/** Which source ended the call, or undefined while it is still running. */
export type AbortCause = "timeout" | "aborted" | undefined;

/** One scorer call's abort signal and what fired it. */
export type Deadline = {
  signal: AbortSignal;
  cause(): AbortCause;
  /** Resolves once `signal` aborts; never rejects. */
  aborted: Promise<void>;
};

/**
 * Starts a deadline of `timeoutMs` linked to `callerSignal`. Never throws:
 * a fractional timeout rounds up, a negative one fires at once, and a
 * non-finite or oversized one is clamped to `MAX_TIMER_MS`, because
 * `AbortSignal.timeout` rejects anything but an in-range integer and a
 * scorer must not throw.
 */
export function startDeadline(
  timeoutMs: number,
  callerSignal: AbortSignal | undefined,
): Deadline {
  const sources = [AbortSignal.timeout(timerMs(timeoutMs))];
  if (callerSignal) sources.push(callerSignal);
  const signal = AbortSignal.any(sources);
  let cause: AbortCause;
  // `any` aborts synchronously from the first source's abort event, so at
  // that moment only that source has aborted.
  const settle = () => {
    cause = callerSignal?.aborted ? "aborted" : "timeout";
  };
  const aborted = new Promise<void>((resolve) => {
    const finish = () => {
      settle();
      resolve();
    };
    if (signal.aborted) finish();
    else signal.addEventListener("abort", finish, { once: true });
  });
  return { signal, cause: () => cause, aborted };
}

function timerMs(timeoutMs: number): number {
  if (!Number.isFinite(timeoutMs)) return MAX_TIMER_MS;
  return Math.min(Math.max(Math.ceil(timeoutMs), 0), MAX_TIMER_MS);
}
