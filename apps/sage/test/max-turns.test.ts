import {
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, expect, it } from "vitest";
import {
  createFaux,
  toolScript,
} from "../../../packages/core/test/support/faux.ts";
import { parseArgs } from "../src/args.ts";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

it("stops after 3 turns when --max-turns 3, including under passthrough", async () => {
  for (const extra of [[], ["--passthrough"]]) {
    dir = mkdtempSync(path.join(tmpdir(), "sage-max-turns-"));
    const { adapter } = createFaux([
      toolScript("c1", "echo"),
      toolScript("c2", "echo"),
      toolScript("c3", "echo"),
      toolScript("c4", "echo"),
    ]);
    const io = capture();
    const code = await runCli(["-p", "x", "--max-turns", "3", ...extra], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapter: adapter as ProviderAdapter,
    });
    expect(code).toBe(1);
    expect(JSON.parse(io.text().stdout)).toMatchObject({
      outcome: "max_turns",
      turns: 3,
    });
    expect(requestHeaderCount(dir)).toBe(3);
    expect(terminalPayload(dir)).toMatchObject({ outcome: "max_turns" });
    rmSync(dir, { recursive: true, force: true });
    dir = "";
  }
});

it("exits 2 when --max-turns is not a positive integer", async () => {
  for (const value of ["abc", "0"]) {
    const io = capture();
    const code = await runCli(["-p", "x", "--max-turns", value], {
      cwd: tmpdir(),
      env: {},
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(2);
    const line = io.text().stderr.trim();
    expect(line).not.toContain("\n");
    expect(line).toContain(value);
    expect(line.toLowerCase()).toContain("max-turns");
    expect(line.toLowerCase()).toMatch(/positive integer/);
    expect(io.text().stdout).toBe("");
  }
});

it("names --max-turns on the usage line", () => {
  const help = parseArgs(["--help"]);
  if (help.ok) throw new Error("help should not parse");
  expect(help.message).toContain("--max-turns <n>");
});

it("bench run --max-turns 60 forwards the flag and records it; the default is 80", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-max-turns-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        id: "one",
        repo: "owner/repo",
        baseCommit: "abc",
        problem: "fix it",
        testCommand: "true",
        timeoutSec: 30,
      },
    ]),
  );
  const capped = path.join(dir, "capped");
  const omitted = path.join(dir, "omitted");
  const passthrough = path.join(dir, "passthrough");
  const cappedArgv: string[][] = [];
  const omittedArgv: string[][] = [];
  const passthroughArgv: string[][] = [];
  const cappedCode = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--max-turns",
      "60",
      "--out",
      capped,
    ],
    benchIo(cappedArgv),
  );
  const omittedCode = await runCli(
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
      omitted,
    ],
    benchIo(omittedArgv),
  );
  const passthroughCode = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "passthrough",
      "--model",
      "claude-test",
      "--max-turns",
      "60",
      "--out",
      passthrough,
    ],
    benchIo(passthroughArgv),
  );
  expect(cappedCode).toBe(0);
  expect(omittedCode).toBe(0);
  expect(passthroughCode).toBe(0);
  expect(adjacentFlag(cappedArgv[0] ?? [], "--max-turns", "60")).toBe(true);
  expect(adjacentFlag(omittedArgv[0] ?? [], "--max-turns", "80")).toBe(true);
  expect(adjacentFlag(passthroughArgv[0] ?? [], "--max-turns", "60")).toBe(
    true,
  );
  expect(readMaxTurns(capped)).toBe(60);
  expect(readMaxTurns(omitted)).toBe(80);
  expect(readMaxTurns(passthrough)).toBe(60);
});

it("bench run exits 2 when --max-turns is not a positive integer", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-max-turns-bad-"));
  for (const value of ["abc", "0"]) {
    const io = capture();
    const code = await runCli(
      [
        "bench",
        "run",
        "--tasks",
        "tasks.json",
        "--policy",
        "compiler",
        "--model",
        "claude-test",
        "--max-turns",
        value,
        "--out",
        "out",
      ],
      {
        cwd: dir,
        env: {},
        stdout: io.stdout,
        stderr: io.stderr,
      },
    );
    expect(code).toBe(2);
    const line = io.text().stderr.trim();
    expect(line).not.toContain("\n");
    expect(line).toContain(value);
    expect(line.toLowerCase()).toContain("max-turns");
    expect(line.toLowerCase()).toMatch(/positive integer/);
    expect(io.text().stdout).toBe("");
  }
});

function benchIo(argv: string[][]) {
  return {
    cwd: dir,
    env: { ANTHROPIC_API_KEY: "test-key" },
    stdout: capture().stdout,
    stderr: capture().stderr,
    benchDeps: {
      async prepare() {},
      async runSage(_task: unknown, _cwd: string, opts: { argv: string[] }) {
        argv.push(opts.argv);
        return { code: 0 };
      },
      async diffEmpty() {
        return true;
      },
      async runTests() {
        return { code: 0 };
      },
    },
  };
}

function adjacentFlag(argv: string[], flag: string, value: string): boolean {
  const index = argv.indexOf(flag);
  return index >= 0 && argv[index + 1] === value;
}

function readMaxTurns(out: string): unknown {
  const parsed: unknown = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  );
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return undefined;
  }
  return "maxTurns" in parsed ? parsed.maxTurns : undefined;
}

function requestHeaderCount(cwd: string): number {
  return sessionEntries(cwd).filter((entry) => entry.kind === "request_header")
    .length;
}

function terminalPayload(cwd: string): { outcome?: string } {
  const terminal = sessionEntries(cwd).findLast(
    (entry) => entry.kind === "terminal",
  );
  if (!terminal || !isRecord(terminal.payload)) {
    throw new Error("missing terminal");
  }
  return terminal.payload as { outcome?: string };
}

function sessionEntries(cwd: string): { kind?: string; payload?: unknown }[] {
  const sessions = readdirSync(path.join(cwd, ".sage", "sessions"));
  const logName = sessions.find((name) => name.endsWith(".jsonl"));
  if (!logName) throw new Error("missing session log");
  return readFileSync(path.join(cwd, ".sage", "sessions", logName), "utf8")
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as { kind?: string; payload?: unknown });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
