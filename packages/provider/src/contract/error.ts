/**
 * Turns a caught value into the text written to tool results, observer
 * errors, and CLI stderr.
 *
 * Guarantees: never throws and never returns undefined.
 *
 * Intentionally not in the Log: nothing here records; callers decide whether
 * the text becomes an `isError` result or a Log field.
 */

/**
 * The `message` of an `Error` (possibly empty), or `String(error)` for any
 * other thrown value. Stack and `cause` are dropped.
 */
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
