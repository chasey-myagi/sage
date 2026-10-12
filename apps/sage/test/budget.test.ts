import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { isRecord } from "@sage/core";
import { createFaux, stopScript } from "@sage/core/testing";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, describe, expect, it } from "vitest";
import { parseArgs } from "../src/args.ts";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

describe("budget flag", () => {
  it("puts --budget 6000 on the first manifest", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-budget-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(["-p", "x", "--budget", "6000"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { anthropic: adapter as ProviderAdapter },
    });
    expect(code).toBe(0);
    const manifest = firstManifest(dir);
    expect(manifest.budget).toEqual(expect.objectContaining({ total: 6000 }));
  });

  it("exits 2 when --budget is not a positive integer", async () => {
    for (const value of ["abc", "0"]) {
      const io = capture();
      const code = await runCli(["-p", "x", "--budget", value], {
        cwd: dir || tmpdir(),
        env: {},
        stdout: io.stdout,
        stderr: io.stderr,
      });
      expect(code).toBe(2);
      const line = io.text().stderr.trim();
      expect(line).not.toContain("\n");
      expect(line).toContain(value);
      expect(line.toLowerCase()).toContain("budget");
      expect(line.toLowerCase()).toMatch(/positive integer/);
      expect(io.text().stdout).toBe("");
    }
  });

  it("ignores --budget under --passthrough and says so once on stderr", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-budget-pass-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(
      ["-p", "x", "--budget", "6000", "--passthrough"],
      {
        cwd: dir,
        env: { ANTHROPIC_API_KEY: "test-key" },
        stdout: io.stdout,
        stderr: io.stderr,
        adapters: { anthropic: adapter as ProviderAdapter },
      },
    );
    expect(code).toBe(0);
    const hints = io
      .text()
      .stderr.split("\n")
      .filter((line) => /budget/i.test(line));
    expect(hints).toEqual([
      "Ignoring --budget because --passthrough does not compile a context",
    ]);
    expect(firstManifest(dir)).not.toHaveProperty("budget");
  });

  it("names --budget on the usage line", () => {
    const help = parseArgs(["--help"]);
    if (help.ok) throw new Error("help should not parse");
    expect(help.message).toContain("--budget <tokens>");
  });
});

function firstManifest(cwd: string): { budget?: { total: number } } {
  const sessions = readdirSync(path.join(cwd, ".sage", "sessions"));
  const logName = sessions.find((name) => name.endsWith(".jsonl"));
  if (!logName) throw new Error("missing session log");
  const entries = readFileSync(
    path.join(cwd, ".sage", "sessions", logName),
    "utf8",
  )
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as { kind?: string; payload?: unknown });
  const manifest = entries.find((entry) => entry.kind === "manifest");
  if (!manifest || !isRecord(manifest.payload)) {
    throw new Error("missing manifest");
  }
  return manifest.payload as { budget?: { total: number } };
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
