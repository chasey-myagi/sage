import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { isRecord } from "@sage/core";
import { createFaux, stopScript, toolScript } from "@sage/core/testing";
import { afterEach, describe, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

/** The task text; a string it already names is never introduced by a chunk. */
const TASK = "find where already_in_prompt is used";

type Call = { id: string; tool: string; input: Record<string, unknown> };

type Entry = {
  seq: number;
  kind: string;
  payload: Record<string, unknown>;
};

function lines(prefix: string, count: number): string {
  return `${Array.from({ length: count }, (_, i) => `${prefix} ${i}`).join("\n")}\n`;
}

function writeWorkspace(): void {
  dir = mkdtempSync(path.join(tmpdir(), "sage-later-use-"));
  writeFileSync(path.join(dir, "a.txt"), lines("a line", 20));
  writeFileSync(path.join(dir, "notes.txt"), "see alpha_marker_x1 here\n");
  writeFileSync(path.join(dir, "other.txt"), "see beta_marker_y2 here\n");
  writeFileSync(path.join(dir, "third.txt"), "already_in_prompt lives here\n");
  mkdirSync(path.join(dir, "src", "deep"), { recursive: true });
  writeFileSync(
    path.join(dir, "src", "deep", "target.ts"),
    "export const needle = 1;\n",
  );
}

/**
 * Records a real compiler session that makes `calls` in order, so the
 * Log, tool results, and manifests all come from the production path.
 * A function gets the workspace root, for calls that name absolute paths.
 */
async function recordSession(
  calls: readonly Call[] | ((root: string) => readonly Call[]),
): Promise<Entry[]> {
  writeWorkspace();
  const script = typeof calls === "function" ? calls(dir) : calls;
  const faux = createFaux([
    ...script.map((call) =>
      toolScript(call.id, call.tool, JSON.stringify(call.input)),
    ),
    stopScript("done"),
  ]);
  const code = await runCli(["--print", TASK], {
    cwd: dir,
    env: { ANTHROPIC_API_KEY: "test-key" },
    stdout: { write() {} },
    stderr: { write() {} },
    adapters: { anthropic: faux.adapter },
  });
  expect(code).toBe(0);
  const sessions = path.join(dir, ".sage", "sessions");
  const [name] = readdirSync(sessions).filter((n) => n.endsWith(".jsonl"));
  if (!name) throw new Error("no session written");
  return readFileSync(path.join(sessions, name), "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as Entry);
}

function hasBlock(entry: Entry, type: string, key: string, id: string) {
  const content = entry.payload.content;
  return (
    entry.kind === "message" &&
    Array.isArray(content) &&
    content.some(
      (block) => isRecord(block) && block.type === type && block[key] === id,
    )
  );
}

/**
 * Copies the Log with a shadow `none` decision for `toolUseId`'s chunk on
 * the first manifest after its result, then returns how `sage stats`
 * labels it: 1 when needed later, 0 when not, per label.
 */
async function labelsOf(
  entries: readonly Entry[],
  toolUseId: string,
): Promise<{ touch: number; content: number }> {
  const call = entries.find((e) => hasBlock(e, "tool_use", "id", toolUseId));
  const result = entries.findIndex((e) =>
    hasBlock(e, "tool_result", "toolUseId", toolUseId),
  );
  if (!call || result < 0) throw new Error(`missing call ${toolUseId}`);
  const chunkId = `seq:${call.seq}#${toolUseId}`;
  const annotated = entries.map((entry) => structuredClone(entry));
  const manifest = annotated.find(
    (entry, index) => index > result && entry.kind === "manifest",
  );
  if (!manifest) throw new Error(`no manifest after ${toolUseId}`);
  manifest.payload.shadow = {
    scorer: "rule",
    calls: 0,
    decisions: [{ id: chunkId, tier: "none", reason: "shadow" }],
  };
  const file = path.join(dir, `audit-${toolUseId}.jsonl`);
  writeFileSync(
    file,
    `${annotated.map((entry) => JSON.stringify(entry)).join("\n")}\n`,
  );
  let stdout = "";
  const code = await runCli(["stats", file], {
    cwd: dir,
    env: {},
    stdout: {
      write(chunk: string) {
        stdout += chunk;
      },
    },
    stderr: { write() {} },
  });
  expect(code).toBe(0);
  const printed = JSON.parse(stdout) as {
    shadowNone: number;
    touch: { laterNeeded: number };
    content: { laterNeeded: number };
  };
  expect(printed.shadowNone).toBe(1);
  return {
    touch: printed.touch.laterNeeded,
    content: printed.content.laterNeeded,
  };
}

describe("later-use labels in sage stats", () => {
  it("credits an edit's old_string to the re-read whose new text holds it, not the earlier read of the file before it changed", async () => {
    const entries = await recordSession([
      { id: "read0", tool: "read", input: { path: "a.txt" } },
      {
        id: "append1",
        tool: "bash",
        input: { command: "seq 5000 5040 >> a.txt" },
      },
      { id: "read2", tool: "read", input: { path: "a.txt" } },
      {
        id: "edit3",
        tool: "edit",
        input: { path: "a.txt", old_string: "5005\n5006", new_string: "x" },
      },
    ]);
    expect(await labelsOf(entries, "read0")).toEqual({ touch: 1, content: 0 });
    // The edit comes in the reply to read2's deciding turn: content counts
    // any call after the decision, touch only calls on later turns.
    expect(await labelsOf(entries, "read2")).toEqual({ touch: 0, content: 1 });
  });

  it("credits a later grep pattern, from the grep tool or rg in bash, to the chunk whose output first showed it", async () => {
    const entries = await recordSession([
      { id: "notes", tool: "read", input: { path: "notes.txt" } },
      { id: "other", tool: "read", input: { path: "other.txt" } },
      { id: "third", tool: "read", input: { path: "third.txt" } },
      { id: "grepA", tool: "grep", input: { pattern: "alpha_marker_x1" } },
      {
        id: "rgB",
        tool: "bash",
        input: { command: "rg -n -e beta_marker_y2 . || true" },
      },
      { id: "grepC", tool: "grep", input: { pattern: "already_in_prompt" } },
    ]);
    expect(await labelsOf(entries, "notes")).toEqual({ touch: 0, content: 1 });
    expect(await labelsOf(entries, "other")).toEqual({ touch: 0, content: 1 });
    // The task already names the pattern, so third.txt did not introduce it.
    expect(await labelsOf(entries, "third")).toEqual({ touch: 0, content: 0 });
  });

  it("marks a bash chunk used when the same command, after normalization, runs again", async () => {
    const entries = await recordSession([
      { id: "run0", tool: "bash", input: { command: "seq 1 1000" } },
      { id: "run1", tool: "bash", input: { command: "seq   1  1000 # again" } },
    ]);
    expect(await labelsOf(entries, "run0")).toEqual({ touch: 0, content: 1 });
    expect(await labelsOf(entries, "run1")).toEqual({ touch: 0, content: 0 });
  });

  it("credits an edit's old_string to the nearest re-read when the file is read twice unchanged", async () => {
    const entries = await recordSession([
      { id: "read0", tool: "read", input: { path: "a.txt" } },
      { id: "read1", tool: "read", input: { path: "a.txt" } },
      {
        id: "edit2",
        tool: "edit",
        input: {
          path: "a.txt",
          old_string: "a line 5\na line 6",
          new_string: "x",
        },
      },
    ]);
    expect(await labelsOf(entries, "read0")).toEqual({ touch: 1, content: 0 });
    expect(await labelsOf(entries, "read1")).toEqual({ touch: 0, content: 1 });
  });

  it("marks a grep used when a path its output named is opened later", async () => {
    const entries = await recordSession([
      { id: "grep0", tool: "grep", input: { pattern: "needle", path: "src" } },
      { id: "read1", tool: "read", input: { path: "src/deep/target.ts" } },
    ]);
    expect(await labelsOf(entries, "grep0")).toEqual({ touch: 0, content: 1 });
  });

  it("matches an absolute read path against the workspace-relative path a grep named", async () => {
    const entries = await recordSession((root) => [
      { id: "grep0", tool: "grep", input: { pattern: "needle", path: "src" } },
      {
        id: "read1",
        tool: "read",
        input: { path: path.join(root, "src", "deep", "target.ts") },
      },
    ]);
    expect(await labelsOf(entries, "grep0")).toEqual({ touch: 0, content: 1 });
  });

  it("counts a bash cat of a path a grep named as opening it", async () => {
    const entries = await recordSession((root) => [
      { id: "grep0", tool: "grep", input: { pattern: "needle", path: "src" } },
      {
        id: "cat1",
        tool: "bash",
        input: { command: `cd ${root} && cat src/deep/target.ts` },
      },
    ]);
    expect(await labelsOf(entries, "grep0")).toEqual({ touch: 0, content: 1 });
  });
});
