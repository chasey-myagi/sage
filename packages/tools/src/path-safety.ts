/**
 * Workspace path check.
 *
 * Contract: a tool path is accepted only when both the lexically resolved
 * path and the realpath (symlink-resolved) stay inside cwd. Callers turn a
 * throw into an isError tool result and must not read or write.
 *
 * Intentionally not in the Log: the resolved path and this error. The model
 * sees only the tool result string, which the session logs.
 */

import { existsSync, lstatSync, readlinkSync, realpathSync } from "node:fs";
import path from "node:path";

export class PathOutsideWorkspaceError extends Error {
  constructor(input: string) {
    super(`Path is outside the workspace: ${input}`);
    this.name = "PathOutsideWorkspaceError";
  }
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

export function resolveInside(cwd: string, input: string): string {
  const lexicalRoot = path.resolve(cwd);
  const lexical = path.resolve(lexicalRoot, input);
  if (!isInside(lexicalRoot, lexical)) {
    throw new PathOutsideWorkspaceError(input);
  }
  let realRoot: string;
  try {
    realRoot = realpathSync(lexicalRoot);
  } catch {
    throw new PathOutsideWorkspaceError(cwd);
  }
  const existing = nearestExisting(lexical);
  let existingReal: string;
  try {
    existingReal = resolvedExisting(existing);
  } catch {
    throw new PathOutsideWorkspaceError(input);
  }
  const remainder = path.relative(existing, lexical);
  const finalPath = path.resolve(existingReal, remainder);
  if (!isInside(realRoot, existingReal) || !isInside(realRoot, finalPath)) {
    throw new PathOutsideWorkspaceError(input);
  }
  if (!existsSync(finalPath)) return finalPath;
  try {
    return realpathSync(finalPath);
  } catch {
    return finalPath;
  }
}

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
