/**
 * Test-process credential strip.
 *
 * Contract: any environment name ending in `_API_KEY` is deleted from the
 * object passed in. The returned list is those names, in enumeration order.
 * Other names are left untouched.
 *
 * Intentionally not in the Log: environment variables and the names removed.
 * A key never becomes an entry.
 */

export function stripSecrets(env: NodeJS.ProcessEnv): string[] {
  const removed: string[] = [];
  for (const key of Object.keys(env)) {
    if (!key.endsWith("_API_KEY")) continue;
    delete env[key];
    removed.push(key);
  }
  return removed;
}
