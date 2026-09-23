/**
 * Builds an Access list from a path argument.
 *
 * Contract: the key is the real absolute path from resolveInside. An outside
 * path returns an empty list and does not throw. Bash does not use this
 * helper; it cannot name its paths statically.
 *
 * Intentionally not in the Log: Access lists. They are a runtime declaration
 * for later concurrency checks, not a second copy of the tool result.
 */

import type { Access } from "@sage/core";
import { asRecord } from "./input.js";
import { resolveInside } from "./path-safety.js";

export function pathAccess(
  cwd: string,
  input: unknown,
  mode: Access["mode"],
  recursive: boolean,
  fallback?: string,
): Access[] {
  const record = asRecord(input);
  if (!record) return [];
  const raw = record.path;
  const target = typeof raw === "string" && raw.length > 0 ? raw : fallback;
  if (target === undefined) return [];
  const resolved = resolveInside(cwd, target);
  if (!resolved.ok) return [];
  return [{ key: resolved.path, mode, recursive }];
}
