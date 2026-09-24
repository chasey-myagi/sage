/**
 * read tool. Returns a page of a UTF-8 workspace file.
 *
 * Contract: default page is 2000 lines and READ_MAX_BYTES bytes, whole lines
 * only. A line that does not fit is not sliced. offset is 1-based.
 *
 * Intentionally not in the Log: file bytes that were not returned. The page
 * and the truncation stub are the tool result the session logs.
 */

import { readFileSync, statSync } from "node:fs";
import { errorText, type HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { asRecord, fail, ok, optionalInt, requiredString } from "./input.js";
import { resolveInside } from "./path-safety.js";
import { splitLines, truncate } from "./truncate.js";

/** Default page length of a read, in lines. */
export const READ_MAX_LINES = 2000;
/** Byte cap of one read page. */
export const READ_MAX_BYTES = 256 * 1024;

const inputSchema = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Workspace path of the file to read.",
    },
    offset: {
      type: "integer",
      minimum: 1,
      description: "1-based line to start at. Defaults to 1.",
    },
    limit: {
      type: "integer",
      minimum: 1,
      description: `Maximum lines to return. Defaults to ${READ_MAX_LINES}.`,
    },
  },
  required: ["path"],
};

/**
 * Creates the read tool bound to `cwd`. A cut page ends with
 * `[truncated: showing lines A-B of N; call read with offset=B+1 to
 * continue]`. A file containing a NUL byte, an offset past the end, and a
 * first line over `READ_MAX_BYTES` are isError results.
 */
export function createRead(cwd: string): HarnessTool {
  return {
    def: {
      name: "read",
      description:
        "Read a UTF-8 text file in the workspace. Use it before editing so the exact text is known.",
      inputSchema,
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses(input) {
      return pathAccess(cwd, input, "read", false);
    },
    async execute(input) {
      try {
        return run(cwd, input);
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}

function run(cwd: string, input: unknown) {
  const record = asRecord(input);
  if (!record) return fail("input must be an object");
  const filePath = requiredString(record, "path");
  if (!filePath) return fail("path must be a string");
  const offsetArg = optionalInt(record, "offset");
  const limitArg = optionalInt(record, "limit");
  if (!offsetArg.ok || !limitArg.ok)
    return fail("offset and limit must be integers");
  const offset = offsetArg.value ?? 1;
  const limit = limitArg.value ?? READ_MAX_LINES;
  if (offset < 1) return fail("offset must be >= 1");
  if (limit < 1) return fail("limit must be >= 1");
  const resolved = resolveInside(cwd, filePath);
  if (!resolved.ok) return resolved.result;
  const abs = resolved.path;
  let text: string;
  try {
    const stat = statSync(abs);
    if (!stat.isFile()) return fail(`not a file: ${filePath}`);
    text = readFileSync(abs, "utf8");
  } catch (error) {
    return fail(errorText(error));
  }
  if (text.includes("\0"))
    return fail(`file is not valid UTF-8 text: ${filePath}`);
  const lines = splitLines(text);
  if (lines.length === 0) {
    if (offset > 1) {
      return fail(`offset ${offset} is past the end of the file (0 lines)`);
    }
    return ok("");
  }
  if (offset > lines.length) {
    return fail(
      `offset ${offset} is past the end of the file (${lines.length} lines)`,
    );
  }
  const window = lines.slice(offset - 1);
  const limited = truncate(window.join("\n"), {
    maxLines: limit,
    maxBytes: READ_MAX_BYTES,
  });
  if (limited.text === "" && window.length > 0) {
    return fail(
      `line ${offset} exceeds the ${READ_MAX_BYTES} byte limit; refusing to return a partial line`,
    );
  }
  const shown = limited.text === "" ? 0 : limited.text.split("\n").length;
  const end = offset - 1 + shown;
  const truncated = end < lines.length || limited.omitted.lines > 0;
  if (!truncated) return ok(limited.text);
  const footer = `[truncated: showing lines ${offset}-${end} of ${lines.length}; call read with offset=${end + 1} to continue]`;
  const content =
    limited.text.length > 0 ? `${limited.text}\n${footer}` : footer;
  return ok(content);
}
