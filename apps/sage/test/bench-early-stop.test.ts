import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { isRecord, JsonlLogStore } from "@sage/core";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

/** One tool call and its result, or a closing text reply. */
type Step = { tool: string; isError?: boolean } | { text: string };

const SESSIONS: Record<string, { steps: Step[]; outcome: string }> = {
  "edit-then-stop": {
    steps: [{ tool: "read" }, { tool: "edit" }, { text: "fixed" }],
    outcome: "done",
  },
  "bash-then-write-then-stop": {
    steps: [{ tool: "bash" }, { tool: "write" }, { text: "fixed" }],
    outcome: "done",
  },
  "edit-then-bash-then-stop": {
    steps: [{ tool: "edit" }, { tool: "bash" }, { text: "fixed" }],
    outcome: "done",
  },
  "failed-edit-after-bash": {
    steps: [
      { tool: "edit" },
      { tool: "bash" },
      { tool: "edit", isError: true },
      { text: "fixed" },
    ],
    outcome: "done",
  },
  "edit-then-failed-bash": {
    steps: [{ tool: "edit" }, { tool: "bash", isError: true }],
    outcome: "done",
  },
  "diff-failed": {
    steps: [{ tool: "edit" }],
    outcome: "done",
  },
  "child-timeout": {
    steps: [{ tool: "edit" }],
    outcome: "done",
  },
  "edit-at-max-turns": {
    steps: [{ tool: "edit" }],
    outcome: "max_turns",
  },
  "edit-then-stop-empty-diff": {
    steps: [{ tool: "edit" }, { text: "fixed" }],
    outcome: "done",
  },
};

it("bench run marks stoppedEarly only when the agent ends its own run after an unverified edit", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const ids = [...Object.keys(SESSIONS), "no-session"];
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify(ids.map(task)));
  const out = path.join(dir, "out");
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "passthrough",
      "--model",
      "claude-test",
      "--out",
      out,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: { write() {} },
      stderr: { write() {} },
      benchDeps: {
        async prepare() {},
        async runSage(spec, cwd) {
          const session = SESSIONS[spec.id];
          if (!session) return { code: 0 };
          return {
            code: 0,
            sessionFile: writeSession(cwd, spec.id, session),
            timedOut: spec.id === "child-timeout",
          };
        },
        async diffEmpty(spec) {
          if (spec.id === "diff-failed") throw new Error("status unavailable");
          return spec.id === "edit-then-stop-empty-diff";
        },
        async runTests() {
          return { code: 1 };
        },
      },
    },
  );

  expect(code).toBe(0);
  const results: unknown = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  );
  if (!isRecord(results) || !Array.isArray(results.tasks))
    throw new Error("Invalid results");
  expect(
    Object.fromEntries(
      results.tasks.map((t) => [
        isRecord(t) ? t.id : undefined,
        isRecord(t) ? t.stoppedEarly : undefined,
      ]),
    ),
  ).toEqual({
    "edit-then-stop": true,
    "bash-then-write-then-stop": true,
    "edit-then-bash-then-stop": false,
    "failed-edit-after-bash": false,
    "edit-then-failed-bash": false,
    "diff-failed": null,
    "child-timeout": null,
    "edit-at-max-turns": false,
    "edit-then-stop-empty-diff": false,
    "no-session": null,
  });
});

it("bench report counts stoppedEarly tasks over the whole task list", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const runA = path.join(dir, "a");
  const runB = path.join(dir, "b");
  writeRun(runA, [true, false, null, false]);
  writeRun(runB, [false, false, false, false]);
  let stdout = "";
  const code = await runCli(["bench", "report", runA, runB], {
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
  const report: unknown = JSON.parse(stdout);
  if (
    !isRecord(report) ||
    !isRecord(report.a) ||
    !isRecord(report.b) ||
    !isRecord(report.delta)
  )
    throw new Error("Invalid report");
  expect(report.a.stoppedEarlyRate).toBe(0.25);
  expect(report.b.stoppedEarlyRate).toBe(0);
  expect(report.delta.stoppedEarlyRate).toBe(-0.25);
});

function task(id: string) {
  return {
    id,
    repo: "owner/repo",
    baseCommit: "abc",
    problem: "problem",
    testCommand: "true",
    timeoutSec: 30,
  };
}

function writeSession(
  cwd: string,
  id: string,
  session: { steps: Step[]; outcome: string },
): string {
  const sessions = path.join(cwd, ".sage", "sessions");
  const store = new JsonlLogStore({ dir: sessions });
  store.append(id, {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "task" }] },
  });
  session.steps.forEach((step, index) => {
    if ("text" in step) {
      store.append(id, {
        kind: "message",
        payload: {
          role: "assistant",
          content: [{ type: "text", text: step.text }],
        },
      });
      return;
    }
    const toolUseId = `call-${index}`;
    store.append(id, {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: toolUseId,
            name: step.tool,
            input: {},
            rawInput: "{}",
          },
        ],
      },
    });
    store.append(id, {
      kind: "message",
      payload: {
        role: "user",
        content: [
          {
            type: "tool_result",
            toolUseId,
            content: "ok",
            isError: step.isError ?? false,
          },
        ],
      },
    });
  });
  store.append(id, { kind: "terminal", payload: { outcome: session.outcome } });
  return path.join(sessions, `${id}.jsonl`);
}

function writeRun(runDir: string, flags: (boolean | null)[]): void {
  mkdirSync(runDir, { recursive: true });
  const tasks = flags.map((stoppedEarly, index) => ({
    id: `t${index}`,
    status: "error",
    reason: "tests-failed",
    health: null,
    stoppedEarly,
  }));
  writeFileSync(
    path.join(runDir, "results.json"),
    JSON.stringify({ policy: "compiler", model: "m", tasks }),
  );
}
