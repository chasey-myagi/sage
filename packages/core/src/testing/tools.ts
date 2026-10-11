/**
 * Inert tool doubles for replaying recorded traces: each tool has a schema
 * and an access list, and `execute` returns an empty success without side
 * effects.
 *
 * Guarantees: `accesses` is a pure function of the input; nothing runs.
 *
 * Intentionally not in the Log: the doubles themselves. A replayed view
 * logs only the tool schemas it puts on the head.
 */

import { isRecord } from "@sage/provider/contract";
import type { Access, HarnessTool } from "../tools.js";

/**
 * A tool whose only access is `input.path` under `mode`; an input without a
 * non-empty string `path` touches nothing. `write` marks it destructive,
 * any other mode read-only. Its schema is `{ name, description: name }`.
 */
export function pathTool(name: string, mode: Access["mode"]): HarnessTool {
  return inertTool(name, mode, (input) => {
    if (!isRecord(input) || typeof input.path !== "string") return [];
    if (input.path.length === 0) return [];
    return [{ key: input.path, mode }];
  });
}

/**
 * A read-only tool that never reports an access, whatever its input, for
 * tools like `bash` whose touches a replay does not track.
 */
export function pathlessTool(name: string): HarnessTool {
  return inertTool(name, "read", () => []);
}

function inertTool(
  name: string,
  mode: Access["mode"],
  accesses: (input: unknown) => Access[],
): HarnessTool {
  return {
    def: {
      name,
      description: name,
      inputSchema: { type: "object" },
    },
    isReadOnly: mode !== "write",
    isDestructive: mode === "write",
    isOpenWorld: false,
    accesses,
    execute() {
      return Promise.resolve({ content: "", isError: false });
    },
  };
}
