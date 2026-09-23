/**
 * edit tool. Replaces one exact occurrence of old_string.
 *
 * Contract: zero matches or two or more matches return isError and do not
 * write. The message states the count. Exactly one match is replaced.
 *
 * Intentionally not in the Log: the file before and after. The status or
 * error string is the tool result the session logs.
 */

import { readFileSync, writeFileSync } from "node:fs";
import type { HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { asRecord, errorText, fail, ok, requiredString } from "./input.js";
import { resolveInside } from "./path-safety.js";

const inputSchema = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace path of the file to edit.",
    },
    old_string: {
      type: "string",
      description: "Exact text to replace. It must occur once.",
    },
    new_string: {
      type: "string",
      description: "Replacement text. May be empty.",
    },
  },
  required: ["path", "old_string", "new_string"],
};

export function createEdit(cwd: string): HarnessTool {
  return {
    def: {
      name: "edit",
      description:
        "Replace one exact occurrence of old_string in a workspace file. Use it for a small in-place change.",
      inputSchema,
    },
    isReadOnly: false,
    isDestructive: true,
    isOpenWorld: false,
    accesses(input) {
      return pathAccess(cwd, input, "write", false);
    },
    async execute(input) {
      try {
        const record = asRecord(input);
        if (!record) return fail("input must be an object");
        const filePath = requiredString(record, "path");
        const oldString = requiredString(record, "old_string");
        const newString = requiredString(record, "new_string");
        if (!filePath) return fail("path must be a string");
        if (oldString === undefined || newString === undefined) {
          return fail("old_string and new_string must be strings");
        }
        if (oldString.length === 0) return fail("old_string must not be empty");
        const resolved = resolveInside(cwd, filePath);
        if (!resolved.ok) return resolved.result;
        const abs = resolved.path;
        let text: string;
        try {
          text = readFileSync(abs, "utf8");
        } catch (error) {
          return fail(errorText(error));
        }
        const count = countOf(text, oldString);
        if (count !== 1) {
          return fail(
            `old_string matched ${count} times; expected exactly one`,
          );
        }
        writeFileSync(abs, text.replace(oldString, newString), "utf8");
        return ok(`Edited ${filePath}`);
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}

function countOf(haystack: string, needle: string): number {
  let count = 0;
  let from = 0;
  while (from <= haystack.length) {
    const at = haystack.indexOf(needle, from);
    if (at < 0) break;
    count += 1;
    from = at + needle.length;
  }
  return count;
}
