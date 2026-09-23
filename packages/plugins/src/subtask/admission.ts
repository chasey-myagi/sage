/**
 * Admission for subtasks: which batches may run in parallel, how many
 * children a session may spawn, and the bounded worker pool.
 *
 * Contract: a batch runs when every task is `readOnly: true`, or when no
 * pair conflicts. A conflict is the same access key with a write on either
 * side. Read-only tasks contribute no accesses. A task that is not
 * read-only and declares no `accesses` is an unknown write and conflicts
 * with every other task. The pool runs at most `limit` tasks at once, and
 * one task throwing does not cancel the others.
 *
 * Intentionally not in the Log: the per-session spawn counters and the
 * pool. A rejected batch is visible only as the tool_result text.
 */

import type { Access } from "@sage/core";
import type { TaskInput } from "./task-input.js";

/**
 * Returns `Access conflict (taskA, taskB, key); …` naming every
 * conflicting pair and key, or undefined when the batch may run. A task
 * is labelled by its trimmed query, else `task<index>`.
 */
export function admissionError(
  tasks: readonly TaskInput[],
): string | undefined {
  if (tasks.every((task) => task.readOnly === true)) return undefined;
  const conflicts: string[] = [];
  for (let left = 0; left < tasks.length; left += 1) {
    for (let right = left + 1; right < tasks.length; right += 1) {
      const taskA = tasks[left];
      const taskB = tasks[right];
      if (!taskA || !taskB) continue;
      for (const key of conflictKeys(taskA, taskB)) {
        conflicts.push(
          `(${taskLabel(taskA, left)}, ${taskLabel(taskB, right)}, ${key})`,
        );
      }
    }
  }
  if (conflicts.length === 0) return undefined;
  return `Access conflict ${conflicts.join("; ")}`;
}

/**
 * Adds `count` to the session's spawn tally when the total stays within
 * `max`, and reports whether it did. A refused reservation adds nothing.
 */
export function reserve(
  spawned: Map<string, number>,
  sessionId: string,
  count: number,
  max: number,
): boolean {
  const used = spawned.get(sessionId) ?? 0;
  if (used + count > max) return false;
  spawned.set(sessionId, used + count);
  return true;
}

/**
 * Runs `run` over `tasks` with at most `limit` in flight and returns the
 * results in task order. A task whose `run` throws gets `onError(error)`
 * in its slot; the others keep running.
 */
export async function runPool<T, R>(
  tasks: readonly T[],
  limit: number,
  run: (task: T) => Promise<R>,
  onError: (error: unknown) => R,
): Promise<R[]> {
  const results = new Array<R>(tasks.length);
  let cursor = 0;
  const workers = Math.max(1, Math.min(limit, tasks.length));
  await Promise.all(
    Array.from({ length: workers }, async () => {
      while (true) {
        const index = cursor;
        cursor += 1;
        if (index >= tasks.length) return;
        const task = tasks[index];
        if (task === undefined) return;
        try {
          results[index] = await run(task);
        } catch (error) {
          results[index] = onError(error);
        }
      }
    }),
  );
  return results;
}

function taskLabel(task: TaskInput, index: number): string {
  const query = task.query.trim();
  return query.length > 0 ? query : `task${index}`;
}

function conflictKeys(left: TaskInput, right: TaskInput): string[] {
  const a = effectiveAccesses(left);
  const b = effectiveAccesses(right);
  if (a === "unknown" || b === "unknown") return ["unknown"];
  if (a === "open" || b === "open") return [];
  return sharedWriteKeys(a, b);
}

function effectiveAccesses(
  task: TaskInput,
): "open" | "unknown" | readonly Access[] {
  if (task.readOnly === true) return "open";
  if (task.accesses === undefined) return "unknown";
  return task.accesses;
}

function sharedWriteKeys(
  left: readonly Access[],
  right: readonly Access[],
): string[] {
  const keys = new Set(left.map((access) => access.key));
  const conflicts: string[] = [];
  for (const key of keys) {
    const leftModes = left
      .filter((access) => access.key === key)
      .map((access) => access.mode);
    const rightModes = right
      .filter((access) => access.key === key)
      .map((access) => access.mode);
    if (leftModes.length === 0 || rightModes.length === 0) continue;
    if ([...leftModes, ...rightModes].includes("write")) conflicts.push(key);
  }
  return conflicts;
}
