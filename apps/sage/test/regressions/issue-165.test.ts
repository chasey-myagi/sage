import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

type Row = { id: string; status: string; reason?: string };

const resolved = (id: string): Row => ({ id, status: "resolved" });
const failed = (id: string, reason: string): Row => ({
  id,
  status: "error",
  reason,
});

it("reports the share of tasks lost to infrastructure faults on each side", async () => {
  const { code, report } = await runReport(
    [
      resolved("t1"),
      failed("t2", "session-error"),
      failed("t3", "tests-failed"),
      resolved("t4"),
    ],
    [
      resolved("t1"),
      resolved("t2"),
      failed("t3", "tests-failed"),
      resolved("t4"),
    ],
  );
  expect(code).toBe(0);
  expect(report.a).toMatchObject({
    faultRate: 0.25,
    errorRate: 0.5,
    resolvedRate: 0.5,
  });
  expect(report.b).toMatchObject({ faultRate: 0, errorRate: 0.25 });
  expect(report.delta).toMatchObject({ faultRate: -0.25 });
  expect(report.excluded).toEqual([]);
});

it("counts harness and upstream failures as faults but not timeouts, test-patch conflicts, failing tests, or unknown reasons", async () => {
  const reasons = [
    "clone-failed",
    "clone-unclean",
    "setup-failed",
    "sage-failed",
    "session-error",
    "session-aborted",
    "diff-failed",
    "timeout",
    "test-patch-failed",
    "tests-failed",
    "hand-written-reason",
  ];
  const rows = reasons.map((reason, index) => failed(`t${index}`, reason));
  const { report } = await runReport(rows, rows);
  expect(report.a).toMatchObject({ faultRate: 7 / 11, errorRate: 1 });
});

it("does not count a fault reason on a row whose status is not error", async () => {
  const { report } = await runReport(
    [{ id: "t1", status: "resolved", reason: "session-error" }],
    [resolved("t1")],
  );
  expect(report.a).toMatchObject({ faultRate: 0, resolvedRate: 1 });
});

it("with --exclude-faults compares only the tasks neither run faulted on and names the dropped ones", async () => {
  const { code, report } = await runReport(
    [
      resolved("t1"),
      failed("t2", "session-error"),
      failed("t3", "tests-failed"),
      resolved("t4"),
      failed("t5", "timeout"),
    ],
    [
      resolved("t1"),
      resolved("t2"),
      resolved("t3"),
      failed("t4", "setup-failed"),
      resolved("t5"),
    ],
    ["--exclude-faults"],
  );
  expect(code).toBe(0);
  expect(report.excluded).toEqual(["t2", "t4"]);
  expect(report.a).toMatchObject({
    resolvedRate: 1 / 3,
    errorRate: 2 / 3,
    faultRate: 0,
  });
  expect(report.b).toMatchObject({
    resolvedRate: 1,
    errorRate: 0,
    faultRate: 0,
  });
  expect(report.delta.resolvedRate).toBeCloseTo(2 / 3);
});

it("accepts --exclude-faults after the run directories", async () => {
  const { code, report } = await runReport(
    [resolved("t1"), failed("t2", "session-aborted")],
    [resolved("t1"), resolved("t2")],
    [],
    ["--exclude-faults"],
  );
  expect(code).toBe(0);
  expect(report.excluded).toEqual(["t2"]);
  expect(report.b).toMatchObject({ resolvedRate: 1 });
});

it("lists a task once when both runs faulted on it", async () => {
  const { report } = await runReport(
    [resolved("t1"), failed("t2", "clone-failed")],
    [resolved("t1"), failed("t2", "session-error")],
    ["--exclude-faults"],
  );
  expect(report.excluded).toEqual(["t2"]);
});

it("prints the usage with the exclude flag for an unknown report flag", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-report-"));
  const io = capture();
  const code = await runCli(["bench", "report", "--drop-faults", "a", "b"], {
    cwd: dir,
    env: {},
    stdout: io.stdout,
    stderr: io.stderr,
  });
  expect(code).toBe(1);
  expect(io.text().stderr).toContain(
    "sage bench report [--exclude-faults] <runA> <runB>",
  );
});

type Report = {
  a: Record<string, unknown>;
  b: Record<string, unknown>;
  delta: Record<string, unknown>;
  excluded: unknown;
};

async function runReport(
  left: Row[],
  right: Row[],
  before: string[] = [],
  after: string[] = [],
): Promise<{ code: number; report: Report }> {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-report-"));
  const runA = path.join(dir, "a");
  const runB = path.join(dir, "b");
  writeResults(runA, left);
  writeResults(runB, right);
  const io = capture();
  const code = await runCli(
    ["bench", "report", ...before, runA, runB, ...after],
    { cwd: dir, env: {}, stdout: io.stdout, stderr: io.stderr },
  );
  const { stdout } = io.text();
  return { code, report: JSON.parse(stdout) as Report };
}

function writeResults(runDir: string, tasks: Row[]): void {
  mkdirSync(runDir, { recursive: true });
  const rows = tasks.map((task) => ({
    ...task,
    health: null,
    stoppedEarly: null,
  }));
  writeFileSync(
    path.join(runDir, "results.json"),
    JSON.stringify({ policy: "compiler", model: "fixture-model", tasks: rows }),
  );
}

function capture() {
  let out = "";
  let err = "";
  return {
    stdout: {
      write(chunk: string) {
        out += chunk;
      },
    },
    stderr: {
      write(chunk: string) {
        err += chunk;
      },
    },
    text: () => ({ stdout: out, stderr: err }),
  };
}
