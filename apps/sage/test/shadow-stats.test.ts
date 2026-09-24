import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isRecord } from "@sage/core";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

const FIXTURE = fileURLToPath(
  new URL(
    "../../../packages/plugins/test/replay/fixtures/issue-63/budget8000-vue10874-36turns.jsonl",
    import.meta.url,
  ),
);

const READ_NONE = "seq:15#call_00_ET_LWwhU0rBRNyCVGSYLNvk5287";
const SPEC_NONE = "seq:115#call_00_emcAMDJIejeBPe5qL6V25115";
const LS_NONE = "seq:3#call_00_FDlEq5R4tzax6XqWMWv30420";
const BASH_NONE = "seq:11#call_00_BZEkWOPBT8CkNNLs5lv32427";
const FULL_ONLY = "seq:7#call_00_bn0A4rmvTEXb1eJZTeEV7592";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

type Entry = {
  id: string;
  parentId: string | null;
  seq: number;
  at: string;
  kind: string;
  payload: Record<string, unknown>;
};

function loadFixture(): Entry[] {
  return readFileSync(FIXTURE, "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as Entry);
}

function addDecision(
  entry: Entry,
  decision: { id: string; tier: string; reason: string },
): void {
  const existing = isRecord(entry.payload.shadow) ? entry.payload.shadow : {};
  const decisions = Array.isArray(existing.decisions)
    ? [...existing.decisions]
    : [];
  decisions.push(decision);
  entry.payload.shadow = { scorer: "rule", calls: 0, decisions };
}

function firstManifestWith(entries: Entry[], chunkId: string): Entry {
  for (const entry of entries) {
    if (entry.kind !== "manifest" || !Array.isArray(entry.payload.chunks)) {
      continue;
    }
    const found = entry.payload.chunks.some(
      (chunk) => isRecord(chunk) && chunk.id === chunkId,
    );
    if (found) return entry;
  }
  throw new Error(`missing chunk ${chunkId}`);
}

function manifestTurn(entries: Entry[], turn: number): Entry {
  const found = entries.find(
    (entry) => entry.kind === "manifest" && entry.payload.turn === turn,
  );
  if (!found) throw new Error(`missing manifest turn ${turn}`);
  return found;
}

function annotate(entries: Entry[]): void {
  for (const id of [READ_NONE, SPEC_NONE, LS_NONE, BASH_NONE]) {
    addDecision(firstManifestWith(entries, id), {
      id,
      tier: "none",
      reason: "shadow",
    });
  }
  addDecision(firstManifestWith(entries, FULL_ONLY), {
    id: FULL_ONLY,
    tier: "full",
    reason: "shadow",
  });
  addDecision(manifestTurn(entries, 10), {
    id: READ_NONE,
    tier: "none",
    reason: "shadow-again",
  });
}

function appendSynthetic(entries: Entry[]): void {
  const last = entries.at(-1);
  if (!last) throw new Error("empty fixture");
  const readSeq = entries.length;
  const read: Entry = {
    id: "00000000-0000-4000-8000-0000000000a1",
    parentId: last.id,
    seq: readSeq,
    at: "2026-09-23T00:00:00.000Z",
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "syn-same-turn",
          name: "read",
          input: { path: "notes/only-once.ts" },
          rawInput: JSON.stringify({ path: "notes/only-once.ts" }),
        },
      ],
    },
  };
  const chunkId = `seq:${readSeq}#syn-same-turn`;
  const manifest: Entry = {
    id: "00000000-0000-4000-8000-0000000000a2",
    parentId: read.id,
    seq: readSeq + 1,
    at: "2026-09-23T00:00:01.000Z",
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 100,
      budget: { total: 8000, used: 1, overrun: 0 },
      claims: [],
      chunks: [{ id: chunkId, tier: "full", reason: "recent", tokens: 1 }],
      anomalies: [],
      shadow: {
        scorer: "rule",
        calls: 0,
        decisions: [{ id: chunkId, tier: "none", reason: "shadow" }],
      },
    },
  };
  const reread: Entry = {
    id: "00000000-0000-4000-8000-0000000000a3",
    parentId: manifest.id,
    seq: readSeq + 2,
    at: "2026-09-23T00:00:02.000Z",
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "syn-same-turn-again",
          name: "read",
          input: { path: "notes/only-once.ts" },
          rawInput: JSON.stringify({ path: "notes/only-once.ts" }),
        },
      ],
    },
  };
  const expand: Entry = {
    id: "00000000-0000-4000-8000-0000000000a4",
    parentId: reread.id,
    seq: readSeq + 3,
    at: "2026-09-23T00:00:03.000Z",
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "syn-expand",
          name: "expand",
          input: { id: LS_NONE },
          rawInput: JSON.stringify({ id: LS_NONE }),
        },
      ],
    },
  };
  entries.push(read, manifest, reread, expand);
}

function capture() {
  let stdout = "";
  let stderr = "";
  return {
    stdout: {
      write(chunk: string) {
        stdout += chunk;
      },
    },
    stderr: {
      write(chunk: string) {
        stderr += chunk;
      },
    },
    text: () => ({ stdout, stderr }),
  };
}

it("reconciles shadow none labels on a real bench trace plus a synthetic expand", async () => {
  const entries = loadFixture();
  annotate(entries);
  appendSynthetic(entries);
  dir = mkdtempSync(path.join(tmpdir(), "sage-shadow-stats-"));
  const file = path.join(dir, "vue10874.jsonl");
  writeFileSync(
    file,
    `${entries.map((entry) => JSON.stringify(entry)).join("\n")}\n`,
  );
  const io = capture();
  const code = await runCli(["stats", file], {
    cwd: dir,
    env: {},
    stdout: io.stdout,
    stderr: io.stderr,
  });
  expect(code).toBe(0);
  const printed = JSON.parse(io.text().stdout.trim()) as {
    shadowNone?: number;
    laterNeeded?: number;
    precision?: number | null;
  };
  expect(printed.shadowNone).toBe(5);
  expect(printed.laterNeeded).toBe(2);
  expect(printed.precision).toBe(1 - 2 / 5);
});

it("bench run forwards --shadow-scorer and rejects an unknown name", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-shadow-bench-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        id: "one",
        repo: "owner/repo",
        baseCommit: "abc",
        problem: "fix",
        testCommand: "true",
        timeoutSec: 30,
      },
    ]),
  );
  const seen: string[][] = [];
  const io = capture();
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--shadow-scorer",
      "model",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare() {},
        async runSage(_task, _cwd, opts) {
          seen.push(opts.argv);
          return { code: 0 };
        },
        async diffEmpty() {
          return true;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(seen[0]).toEqual(expect.arrayContaining(["--shadow-scorer", "model"]));
  const bad = capture();
  const rejected = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--shadow-scorer",
      "nope",
      "--out",
      path.join(dir, "bad"),
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: bad.stdout,
      stderr: bad.stderr,
    },
  );
  expect(rejected).toBe(1);
  expect(bad.text().stderr).toContain("--shadow-scorer");
});
