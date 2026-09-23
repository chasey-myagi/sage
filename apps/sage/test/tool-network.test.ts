import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { JsonlLogStore } from "@sage/core";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, expect, it } from "vitest";
import {
  createFaux,
  stopScript,
  toolScript,
} from "../../../packages/core/test/support/faux.ts";
import { parseArgs } from "../src/args.ts";
import { createCodingSession } from "../src/coding-session.ts";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

it("exits 2 for network none on a fake non-darwin platform", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-net-platform-"));
  const win = capture();
  const winCode = await runCli(["-p", "x", "--tool-network", "none"], {
    cwd: dir,
    env: {},
    stdout: win.stdout,
    stderr: win.stderr,
    platform: "win32",
  });
  expect(winCode).toBe(2);
  expect(win.text().stdout).toBe("");
  expect(win.text().stderr).toBe(
    "tool network none is not supported on win32\n",
  );

  const linux = capture();
  const linuxCode = await runCli(["-p", "x", "--tool-network", "none"], {
    cwd: dir,
    env: {},
    stdout: linux.stdout,
    stderr: linux.stderr,
    platform: "linux",
    unshareAvailable: false,
  });
  expect(linuxCode).toBe(2);
  expect(linux.text().stdout).toBe("");
  expect(linux.text().stderr).toBe(
    "tool network none requires unshare -n, which is not available\n",
  );
});

it("rejects an unknown --tool-network value with exit 2", async () => {
  const parsed = parseArgs(["-p", "x", "--tool-network", "maybe"]);
  expect(parsed).toEqual({
    ok: false,
    exitCode: 2,
    message: 'Invalid --tool-network "maybe": expected none or full',
  });
  const io = capture();
  const code = await runCli(["-p", "x", "--tool-network", "maybe"], {
    cwd: tmpdir(),
    env: {},
    stdout: io.stdout,
    stderr: io.stderr,
  });
  expect(code).toBe(2);
  expect(io.text().stderr).toBe(
    'Invalid --tool-network "maybe": expected none or full\n',
  );
  const help = parseArgs(["--help"]);
  if (help.ok) throw new Error("help should not parse");
  expect(help.message).toContain("--tool-network none|full");
  expect(help.message).toContain("--tool-sandbox none|full");
});

it("bench run defaults to tool network none and can record full", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-net-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        id: "one",
        repo: "owner/repo",
        baseCommit: "abc",
        problem: "LEAKED_PROBLEM_TEXT",
        testCommand: "true",
        timeoutSec: 30,
      },
    ]),
  );
  const noneOut = path.join(dir, "none");
  const fullOut = path.join(dir, "full");
  const noneArgv: string[][] = [];
  const fullArgv: string[][] = [];
  const none = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--out",
      noneOut,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: capture().stdout,
      stderr: capture().stderr,
      benchDeps: deps(noneArgv),
    },
  );
  const full = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--tool-network",
      "full",
      "--out",
      fullOut,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: capture().stdout,
      stderr: capture().stderr,
      benchDeps: deps(fullArgv),
    },
  );
  expect(none).toBe(0);
  expect(noneArgv[0] ?? []).toEqual(
    expect.arrayContaining(["--tool-network", "none"]),
  );
  expect(adjacent(noneArgv[0] ?? [], "none")).toBe(true);
  expect(readToolNetwork(noneOut)).toBe("none");
  expect(full).toBe(0);
  expect(fullArgv[0] ?? []).toEqual(
    expect.arrayContaining(["--tool-network", "full"]),
  );
  expect(readToolNetwork(fullOut)).toBe("full");
});

it("logs network none on the bash tool result and not in model-visible text", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-net-log-"));
  const { adapter } = createFaux([
    toolScript("bash1", "bash", JSON.stringify({ command: "echo hi" })),
    stopScript("done"),
  ]);
  const { session, store, sessionId } = createCodingSession({
    cwd: dir,
    adapter: adapter as ProviderAdapter,
    model: { provider: "anthropic", model: "claude-test" },
    passthrough: true,
    sandbox: { network: "none" },
    platform: "darwin",
    exec: async () => ({ stdout: "ok\n", stderr: "", code: 0 }),
  });
  const summary = await session.run("x");
  expect(summary.outcome).toBe("done");
  const listed = store.path(sessionId).flatMap((entry) => {
    const details = entry.details as { toolResults?: unknown } | undefined;
    return Array.isArray(details?.toolResults) ? details.toolResults : [];
  });
  expect(listed).toContainEqual({
    toolUseId: "bash1",
    name: "bash",
    details: { network: "none" },
  });
  const visible = store
    .path(sessionId)
    .filter((entry) => entry.kind === "message")
    .map((entry) => JSON.stringify(entry.payload));
  expect(visible.join("\n")).not.toContain('"network"');
  expect(visible.join("\n")).toContain("[bash exit code 0]");
});

function deps(argv: string[][]) {
  return {
    async prepare() {},
    async runSage(
      _task: { id: string },
      cwd: string,
      opts: { argv: string[] },
    ) {
      argv.push(opts.argv);
      return { code: 0, sessionFile: writeSession(cwd, "one") };
    },
    async diffEmpty() {
      return false;
    },
    async runTests() {
      return { code: 0 };
    },
  };
}

function writeSession(cwd: string, id: string): string {
  const sessions = path.join(cwd, ".sage", "sessions");
  const store = new JsonlLogStore({ dir: sessions });
  store.append(id, {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "task" }],
    },
  });
  store.append(id, {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "done" }],
    },
    details: {
      usage: {
        inputOther: 3,
        inputCacheRead: 1,
        inputCacheCreation: 0,
        output: 2,
      },
    },
  });
  store.append(id, {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      budget: { total: 10, used: 2, overrun: 0, omitted: 4, expands: 1 },
      claims: [],
      chunks: [],
      anomalies: [],
    },
  });
  return path.join(sessions, `${id}.jsonl`);
}

function adjacent(argv: string[], value: string): boolean {
  const index = argv.indexOf("--tool-network");
  return index >= 0 && argv[index + 1] === value;
}

function readToolNetwork(out: string): unknown {
  const parsed: unknown = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  );
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return undefined;
  }
  return "toolNetwork" in parsed ? parsed.toolNetwork : undefined;
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
    text() {
      return { stdout, stderr };
    },
  };
}
