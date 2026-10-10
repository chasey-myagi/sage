/**
 * Early-stop detection: did the agent end its own run right after changing
 * files it never checked?
 *
 * Contract: `endedAfterUnverifiedEdit` is a pure function of one session
 * path. It reads the terminal outcome, tool_use names, and tool_result
 * `isError` flags; it never parses bash command text or tool output. The
 * bench combines it with a non-empty diff to set `stoppedEarly`. Counting
 * only: nothing here feeds back into the agent.
 *
 * Intentionally not in the Log: the flag. It lives in `results.json`.
 */

import { isMessage, isRecord, type StoredEntry } from "@sage/core";
import { TOOL_NAMES } from "@sage/tools";

const EDIT_TOOLS: ReadonlySet<string> = new Set([
  TOOL_NAMES.edit,
  TOOL_NAMES.write,
]);

/**
 * True when the terminal outcome is `done` and the last successful `edit`
 * or `write` result comes after the last `bash` result, or no `bash` ran
 * at all. A failed bash still counts as a check; a failed edit changes
 * nothing and is ignored. False for a path without a successful edit.
 */
export function endedAfterUnverifiedEdit(
  path: readonly StoredEntry[],
): boolean {
  if (terminalOutcome(path) !== "done") return false;
  const names = new Map<string, string>();
  let order = 0;
  let lastEdit = -1;
  let lastBash = -1;
  for (const entry of path) {
    if (entry.kind !== "message" || !isMessage(entry.payload)) continue;
    for (const block of entry.payload.content) {
      if (!isRecord(block)) continue;
      if (block.type === "tool_use") {
        if (typeof block.id === "string" && typeof block.name === "string") {
          names.set(block.id, block.name);
        }
        continue;
      }
      if (block.type !== "tool_result" || typeof block.toolUseId !== "string") {
        continue;
      }
      order += 1;
      const name = names.get(block.toolUseId);
      if (name === TOOL_NAMES.bash) lastBash = order;
      else if (name && EDIT_TOOLS.has(name) && block.isError !== true) {
        lastEdit = order;
      }
    }
  }
  return lastEdit > lastBash;
}

function terminalOutcome(path: readonly StoredEntry[]): string | undefined {
  const last = path.at(-1);
  if (last?.kind !== "terminal" || !isRecord(last.payload)) return undefined;
  return typeof last.payload.outcome === "string"
    ? last.payload.outcome
    : undefined;
}
