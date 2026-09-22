/**
 * grep tool. Searches workspace text with a regular expression.
 *
 * Contract: uses ripgrep when `rg` is on PATH, otherwise walks files in
 * process. Hits are `file:line: text`, capped at 200 unless max_results is
 * set. Paths that escape the workspace return isError.
 *
 * Intentionally not in the Log: files that did not match. The hit list is
 * the tool result the session logs.
 */

import { spawn } from "node:child_process";
import { readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { globToRegExp } from "./glob.js";
import {
  asRecord,
  errorText,
  fail,
  ok,
  optionalInt,
  optionalString,
  requiredString,
  type ToolResult,
} from "./input.js";
import { resolveInside, workspaceRelative } from "./path-safety.js";

export const GREP_MAX_RESULTS = 200;
const MAX_FILE_BYTES = 1_000_000;
const SKIP_DIRS = new Set([".git", ".sage", "node_modules"]);

const inputSchema = {
  type: "object",
  properties: {
    pattern: {
      type: "string",
      description: "Regular expression to search for.",
    },
    path: {
      type: "string",
      description: "File or directory to search. Defaults to the workspace.",
    },
    glob: {
      type: "string",
      description: "Optional glob that limits which files are searched.",
    },
    max_results: {
      type: "integer",
      minimum: 1,
      description: "Maximum hits to return. Defaults to 200.",
    },
  },
  required: ["pattern"],
};

type Hit = { file: string; line: number; text: string };

let rgReady: Promise<boolean> | undefined;

export function createGrep(cwd: string): HarnessTool {
  return {
    def: {
      name: "grep",
      description:
        "Search file contents with a regular expression. Use it to find where a string is used.",
      inputSchema,
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses(input) {
      return pathAccess(cwd, input, "search", true, ".");
    },
    async execute(input) {
      try {
        const record = asRecord(input);
        if (!record) return fail("input must be an object");
        const pattern = requiredString(record, "pattern");
        if (!pattern) return fail("pattern must be a string");
        const pathArg = optionalString(record, "path");
        const globArg = optionalString(record, "glob");
        const maxArg = optionalInt(record, "max_results");
        if (!pathArg.ok || !globArg.ok || !maxArg.ok) {
          return fail("path, glob, and max_results have the wrong type");
        }
        const maxResults = maxArg.value ?? GREP_MAX_RESULTS;
        if (maxResults < 1) return fail("max_results must be >= 1");
        const root = resolveInside(cwd, pathArg.value ?? ".");
        if (await hasRg()) {
          try {
            const hits = await searchWithRg({
              cwd,
              pattern,
              root,
              glob: globArg.value,
            });
            return ok(formatHits(hits, maxResults));
          } catch (error) {
            if (!isMissing(error)) return fail(errorText(error));
          }
        }
        return searchWithoutRg({
          cwd,
          pattern,
          root,
          glob: globArg.value,
          maxResults,
        });
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}

export async function searchWithoutRg(opts: {
  cwd: string;
  pattern: string;
  root: string;
  glob?: string;
  maxResults: number;
}): Promise<ToolResult> {
  let regex: RegExp;
  try {
    regex = new RegExp(opts.pattern);
  } catch (error) {
    return fail(errorText(error));
  }
  let matcher: RegExp | undefined;
  if (opts.glob) {
    try {
      matcher = globToRegExp(opts.glob);
    } catch (error) {
      return fail(errorText(error));
    }
  }
  const root = resolveInside(opts.cwd, opts.root);
  const hits = collectHits(opts.cwd, root, regex, matcher);
  return ok(formatHits(hits, opts.maxResults));
}

function collectHits(
  cwd: string,
  root: string,
  regex: RegExp,
  glob: RegExp | undefined,
): Hit[] {
  const hits: Hit[] = [];
  let rootStat: ReturnType<typeof statSync>;
  try {
    rootStat = statSync(root);
  } catch {
    return hits;
  }
  if (rootStat.isFile()) {
    pushFile(hits, cwd, root, regex, glob);
    return hits;
  }
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current) break;
    let entries: {
      name: string;
      isSymbolicLink(): boolean;
      isDirectory(): boolean;
      isFile(): boolean;
    }[];
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (SKIP_DIRS.has(entry.name)) continue;
      const abs = path.join(current, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        stack.push(abs);
        continue;
      }
      if (!entry.isFile()) continue;
      pushFile(hits, cwd, abs, regex, glob);
    }
  }
  hits.sort((left, right) => {
    if (left.file !== right.file) return left.file < right.file ? -1 : 1;
    return left.line - right.line;
  });
  return hits;
}

function pushFile(
  hits: Hit[],
  cwd: string,
  abs: string,
  regex: RegExp,
  glob: RegExp | undefined,
): void {
  const rel = workspaceRelative(cwd, abs);
  if (glob && !glob.test(rel)) return;
  let stat: ReturnType<typeof statSync>;
  try {
    stat = statSync(abs);
  } catch {
    return;
  }
  if (stat.size > MAX_FILE_BYTES) return;
  let text: string;
  try {
    text = readFileSync(abs, "utf8");
  } catch {
    return;
  }
  if (text.includes("\0")) return;
  const lines = text.split("\n");
  if (text.endsWith("\n")) lines.pop();
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? "";
    if (!regex.test(line)) continue;
    hits.push({ file: rel, line: index + 1, text: line });
  }
}

export function formatHits(hits: Hit[], maxResults: number): string {
  if (hits.length === 0) return "No matches.";
  const shown = hits.slice(0, maxResults);
  const lines = shown.map((hit) => `${hit.file}:${hit.line}: ${hit.text}`);
  if (hits.length > maxResults) {
    lines.push(
      `[truncated: showing ${maxResults} of ${hits.length} matches; narrow the path or raise max_results]`,
    );
  }
  return lines.join("\n");
}

function hasRg(): Promise<boolean> {
  rgReady ??= new Promise((resolve) => {
    const child = spawn("rg", ["--version"], { stdio: "ignore" });
    child.on("error", () => resolve(false));
    child.on("close", (code) => resolve(code === 0));
  });
  return rgReady;
}

function isMissing(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    (error as { code?: string }).code === "ENOENT"
  );
}

async function searchWithRg(opts: {
  cwd: string;
  pattern: string;
  root: string;
  glob?: string;
}): Promise<Hit[]> {
  const workspace = realpathSync(opts.cwd);
  let relRoot = path.relative(workspace, opts.root);
  if (relRoot === "") relRoot = ".";
  const args = [
    "-n",
    "-H",
    "--no-heading",
    "--color",
    "never",
    "--glob",
    "!.git/**",
    "--glob",
    "!.sage/**",
    "--glob",
    "!node_modules/**",
    "-e",
    opts.pattern,
  ];
  if (opts.glob) args.push("--glob", opts.glob);
  args.push(relRoot);
  const output = await runRg(args, workspace);
  const hits: Hit[] = [];
  for (const raw of output.split("\n")) {
    if (raw.length === 0) continue;
    const match = /^([^:]+):(\d+):(.*)$/.exec(raw);
    if (!match) continue;
    const file = (match[1] ?? "")
      .replace(/^\.\//, "")
      .split(path.sep)
      .join("/");
    const line = Number(match[2]);
    const text = match[3] ?? "";
    if (!Number.isInteger(line)) continue;
    hits.push({ file, line, text });
  }
  return hits;
}

function runRg(args: string[], cwd: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn("rg", args, { cwd });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout?.on("data", (chunk: Buffer) => stdout.push(chunk));
    child.stderr?.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0 || code === 1) {
        resolve(Buffer.concat(stdout).toString("utf8"));
        return;
      }
      reject(
        new Error(
          Buffer.concat(stderr).toString("utf8") || `rg exited ${code}`,
        ),
      );
    });
  });
}
