/**
 * Workspace path check.
 *
 * Contract: a tool path is accepted only when both the lexically resolved
 * path and the realpath (symlink-resolved) stay inside cwd. An outside path
 * returns `{ ok: false, result }` where `result` is `fail` with content
 * `[error] Path is outside the workspace: <input>`. It does not throw.
 * Callers must not read or write that path.
 *
 * Intentionally not in the Log: the resolved path. The model sees only the
 * tool result string, which the session logs.
 */

import { existsSync, lstatSync, readlinkSync, realpathSync } from "node:fs";
import path from "node:path";
import { SAGE_DIR_NAME } from "@sage/core";
import { fail, type ToolResult } from "./input.js";

/** Directory names find and grep never descend into. */
export const SKIPPED_DIR_NAMES: ReadonlySet<string> = new Set([
  ".git",
  SAGE_DIR_NAME,
  "node_modules",
]);

/** A path cleared for use, or the isError result to return instead. */
export type ResolvedPath =
  | { ok: true; path: string }
  | { ok: false; result: ToolResult };

function outside(input: string): ResolvedPath {
  return {
    ok: false,
    result: fail(`[error] Path is outside the workspace: ${input}`),
  };
}

function isInside(root: string, target: string): boolean {
  const relative = path.relative(root, target);
  if (relative === "") return true;
  if (path.isAbsolute(relative)) return false;
  return !relative.split(path.sep).includes("..");
}

function existsOrLink(target: string): boolean {
  try {
    lstatSync(target);
    return true;
  } catch {
    return false;
  }
}

function nearestExisting(target: string): string {
  let current = target;
  for (;;) {
    if (existsOrLink(current)) return current;
    const parent = path.dirname(current);
    if (parent === current) return current;
    current = parent;
  }
}

function resolvedExisting(target: string): string {
  try {
    return realpathSync(target);
  } catch (error) {
    let stat: ReturnType<typeof lstatSync>;
    try {
      stat = lstatSync(target);
    } catch {
      throw error;
    }
    if (!stat.isSymbolicLink()) throw error;
    const link = readlinkSync(target);
    return path.resolve(path.dirname(target), link);
  }
}

/**
 * Resolves `input` against `cwd` and checks both the lexical path and the
 * realpath of its nearest existing ancestor stay inside. The returned path
 * is the realpath when the target exists, else the resolved would-be path.
 * A dangling symlink is followed by its link text.
 */
export function resolveInside(cwd: string, input: string): ResolvedPath {
  const lexicalRoot = path.resolve(cwd);
  const lexical = path.resolve(lexicalRoot, input);
  if (!isInside(lexicalRoot, lexical)) return outside(input);
  let realRoot: string;
  try {
    realRoot = realpathSync(lexicalRoot);
  } catch {
    return outside(cwd);
  }
  const existing = nearestExisting(lexical);
  let existingReal: string;
  try {
    existingReal = resolvedExisting(existing);
  } catch {
    return outside(input);
  }
  const remainder = path.relative(existing, lexical);
  const finalPath = path.resolve(existingReal, remainder);
  if (!isInside(realRoot, existingReal) || !isInside(realRoot, finalPath)) {
    return outside(input);
  }
  if (!existsSync(finalPath)) return { ok: true, path: finalPath };
  try {
    return { ok: true, path: realpathSync(finalPath) };
  } catch {
    return { ok: true, path: finalPath };
  }
}

/**
 * `abs` as a posix path relative to the workspace realpath (`.` for the
 * root). A path outside the workspace comes back absolute. Throws when
 * `cwd` does not exist.
 */
export function workspaceRelative(cwd: string, abs: string): string {
  const base = realpathSync(path.resolve(cwd));
  let target = abs;
  try {
    if (existsSync(abs)) target = realpathSync(abs);
  } catch {
    target = abs;
  }
  const relative = path.relative(base, target);
  if (relative === "") return ".";
  if (path.isAbsolute(relative) || relative.split(path.sep).includes("..")) {
    return target.split(path.sep).join("/");
  }
  return relative.split(path.sep).join("/");
}
