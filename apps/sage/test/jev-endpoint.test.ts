import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createFaux, stopScript, toolScript } from "@sage/core/testing";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, describe, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

type Seen = {
  url: string;
  authorization?: string;
  model?: string;
};

function capture() {
  let stderr = "";
  return {
    stdout: {
      write(_chunk: string) {},
    },
    stderr: {
      write(chunk: string) {
        stderr += chunk;
      },
    },
    text() {
      return stderr;
    },
  };
}

function recordFetch() {
  const seen: Seen[] = [];
  const fetchImpl = async (
    url: string,
    init?: { headers?: Record<string, string>; body?: string },
  ) => {
    let model: string | undefined;
    if (init?.body) {
      const parsed = JSON.parse(init.body) as { model?: unknown };
      if (typeof parsed.model === "string") model = parsed.model;
    }
    seen.push({
      url,
      authorization: init?.headers?.Authorization,
      model,
    });
    return new Response(
      JSON.stringify({ answers: {}, usage: { input_tokens: 1 } }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  };
  return { seen, fetchImpl };
}

// Entry scoring is what makes this two-turn fixture call jev. Rebuild
// scoring waits for an epoch a short run never opens. Shadow scoring
// runs on any new tool chunk and must not use --scorer-apply entry.
async function runScored(opts: {
  args: string[];
  env: NodeJS.ProcessEnv;
  body: string;
}) {
  dir = mkdtempSync(path.join(tmpdir(), "sage-jev-endpoint-"));
  writeFileSync(path.join(dir, "page.txt"), opts.body);
  const { adapter } = createFaux([
    toolScript("read1", "read", JSON.stringify({ path: "page.txt" })),
    stopScript("done"),
  ]);
  const io = capture();
  const recorded = recordFetch();
  const code = await runCli(opts.args, {
    cwd: dir,
    env: opts.env,
    stdout: io.stdout,
    stderr: io.stderr,
    adapter: adapter as ProviderAdapter,
    jevFetch: recorded.fetchImpl,
  });
  return { code, stderr: io.text(), seen: recorded.seen };
}

describe("jev endpoint", () => {
  it("posts --scorer jev to JEV_BASE_URL with JEV_API_KEY and JEV_MODEL", async () => {
    const result = await runScored({
      args: [
        "--print",
        "read page",
        "--scorer",
        "jev",
        "--scorer-apply",
        "entry",
      ],
      env: {
        ANTHROPIC_API_KEY: "test-key",
        JEV_BASE_URL: "http://127.0.0.1:30000",
        JEV_API_KEY: "x",
        JEV_MODEL: "served-model",
      },
      body: "A".repeat(2000),
    });
    expect(result.code).toBe(0);
    expect(result.seen).toEqual([
      {
        url: "http://127.0.0.1:30000/v1/systemone",
        authorization: "Bearer x",
        model: "served-model",
      },
    ]);
  });

  it("posts --shadow-scorer jev to JEV_BASE_URL with JEV_API_KEY and JEV_MODEL", async () => {
    const result = await runScored({
      args: ["--print", "read page", "--shadow-scorer", "jev"],
      env: {
        ANTHROPIC_API_KEY: "test-key",
        JEV_BASE_URL: "http://127.0.0.1:30000",
        JEV_API_KEY: "x",
        JEV_MODEL: "served-model",
      },
      body: "hi\n",
    });
    expect(result.code).toBe(0);
    expect(result.seen).toEqual([
      {
        url: "http://127.0.0.1:30000/v1/systemone",
        authorization: "Bearer x",
        model: "served-model",
      },
    ]);
  });

  it("keeps the TypeSafe request when only TYPESAFE_API_KEY is set", async () => {
    const result = await runScored({
      args: [
        "--print",
        "read page",
        "--scorer",
        "jev",
        "--scorer-apply",
        "entry",
      ],
      env: {
        ANTHROPIC_API_KEY: "test-key",
        TYPESAFE_API_KEY: "typesafe-secret",
      },
      body: "A".repeat(2000),
    });
    expect(result.code).toBe(0);
    expect(result.seen).toEqual([
      {
        url: "https://api.typesafe.ai/v1/systemone",
        authorization: "Bearer typesafe-secret",
        model: "jev-latest",
      },
    ]);
  });

  it("exits 2 and names both key variables when neither is set", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-jev-nokey-"));
    for (const flag of ["--scorer", "--shadow-scorer"] as const) {
      const io = capture();
      const code = await runCli(["--print", "hi", flag, "jev"], {
        cwd: dir,
        env: {},
        stdout: io.stdout,
        stderr: io.stderr,
      });
      expect(code).toBe(2);
      expect(io.text()).toContain("JEV_API_KEY");
      expect(io.text()).toContain("TYPESAFE_API_KEY");
    }
  });
});
