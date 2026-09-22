/**
 * write tool. Creates or overwrites one UTF-8 file inside the workspace.
 *
 * Contract: the path is resolveInside'd before any write. Parent directories
 * are created. An outside path or an escaping symlink returns isError.
 *
 * Intentionally not in the Log: the bytes written to disk. The short status
 * string is the tool result the session logs.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { asRecord, errorText, fail, ok, requiredString } from "./input.js";
import { resolveInside } from "./path-safety.js";

const inputSchema = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace path of the file to write.",
    },
    content: {
      type: "string",
      description: "Full UTF-8 contents to write.",
    },
  },
  required: ["path", "content"],
};

export function createWrite(cwd: string): HarnessTool {
  return {
    def: {
      name: "write",
      description:
        "Create or overwrite a UTF-8 file in the workspace. Use it for new files or a full rewrite.",
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
        const content = requiredString(record, "content");
        if (!filePath) return fail("path must be a string");
        if (content === undefined) return fail("content must be a string");
        const abs = resolveInside(cwd, filePath);
        mkdirSync(path.dirname(abs), { recursive: true });
        writeFileSync(abs, content, "utf8");
        return ok(`Wrote ${filePath}`);
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}
