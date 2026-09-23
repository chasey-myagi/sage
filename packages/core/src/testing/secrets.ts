/**
 * Test-process credential strip.
 *
 * Guarantees: any environment name ending in `_API_KEY` is deleted from the
 * object passed in. The returned list is those names, in enumeration order.
 * Other names are left untouched.
 *
 * Intentionally not in the Log: environment variables and the names removed.
 * A key never becomes an entry.
 */

/** Deletes every `*_API_KEY` from `env` in place; returns the names removed. */
export function stripSecrets(env: NodeJS.ProcessEnv): string[] {
  const removed: string[] = [];
  for (const key of Object.keys(env)) {
    if (!key.endsWith("_API_KEY")) continue;
    delete env[key];
    removed.push(key);
  }
  return removed;
}
