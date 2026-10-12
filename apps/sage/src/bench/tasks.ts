/**
 * Reads the bench tasks file.
 *
 * Contract: the file is a JSON array of {@link BenchTask}. Any malformed
 * entry throws with its index, before any repository is fetched. Empty
 * optional strings are treated as absent.
 *
 * Intentionally not in the Log: task text. The problem reaches the Log only
 * as the user message of the child `sage -p` run.
 */

import { readFileSync } from "node:fs";
import { isFiniteNumber, isRecord } from "@sage/core";
import type { BenchTask } from "./types.js";

/**
 * Loads and validates a tasks file. Throws on unreadable JSON, a non-array,
 * or an entry missing a required field or with a non-positive `timeoutSec`.
 */
export function loadTasks(file: string): BenchTask[] {
  const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(parsed))
    throw new Error("tasks file must be a JSON array");
  return parsed.map((item, index) => parseTask(item, index));
}

function parseTask(value: unknown, index: number): BenchTask {
  if (!isRecord(value)) throw new Error(`tasks[${index}] is not an object`);
  const timeoutSec = value.timeoutSec;
  if (!isFiniteNumber(timeoutSec) || timeoutSec <= 0) {
    throw new Error(`tasks[${index}] timeoutSec must be a positive number`);
  }
  const setupCommand = optionalString(value, "setupCommand", index);
  const testPatch = optionalString(value, "testPatch", index);
  return {
    id: requiredString(value, "id", index),
    repo: requiredString(value, "repo", index),
    baseCommit: requiredString(value, "baseCommit", index),
    problem: requiredString(value, "problem", index),
    testCommand: requiredString(value, "testCommand", index),
    timeoutSec,
    ...(setupCommand ? { setupCommand } : {}),
    ...(testPatch ? { testPatch } : {}),
  };
}

function optionalString(
  value: Record<string, unknown>,
  key: string,
  index: number,
): string | undefined {
  if (!Object.hasOwn(value, key) || value[key] === undefined) return undefined;
  const found = value[key];
  if (typeof found !== "string") {
    throw new Error(`tasks[${index}] ${key} must be a string`);
  }
  if (found.length === 0) return undefined;
  return found;
}

function requiredString(
  value: Record<string, unknown>,
  key: string,
  index: number,
): string {
  const found = value[key];
  if (typeof found !== "string" || found.length === 0) {
    throw new Error(`tasks[${index}] missing ${key}`);
  }
  return found;
}
