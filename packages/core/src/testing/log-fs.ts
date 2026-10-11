/**
 * `LogFs` doubles for `JsonlLogStore` tests.
 *
 * Guarantees: every method not overridden is the store's production
 * `LogFs`, so a test spells out only the calls it intercepts and the rest
 * runs exactly as in production.
 *
 * Intentionally not in the Log: the doubles and whatever they count or fail.
 */

import { type LogFs, nodeFs } from "../log/jsonl.js";

/**
 * Returns the production `LogFs` with `overrides` replacing the named
 * methods. Each call returns a fresh object; the overrides are used as
 * given, not wrapped.
 */
export function nodeLogFs(overrides: Partial<LogFs> = {}): LogFs {
  return { ...nodeFs, ...overrides };
}
