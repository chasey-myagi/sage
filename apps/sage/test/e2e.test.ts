import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createFaux, stopScript, toolScript } from "@sage/core/testing";
import {
  NO_LOCAL_PROVIDERS,
  OpenAIAdapter,
  type ProviderAdapter,
} from "@sage/provider";
import { afterEach, describe, expect, it } from "vitest";
import { parseArgs } from "../src/args.ts";
import { createCodingSession } from "../src/coding-session.ts";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
});

describe("health output", () => {
  it("rejects --json like any unknown flag", async () => {
    const unknown = parseArgs(["--print", "hi", "--nope"]);
    expect(unknown.ok).toBe(false);
    expect(parseArgs(["--print", "hi", "--json"])).toEqual(unknown);
    expect(parseArgs(["-p", "--json", "hi"])).toEqual(unknown);
    const io = capture();
    const code = await runCli(["-p", "hi", "--json"], {
      cwd: tmpdir(),
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(1);
    expect(io.text().stdout).toBe("");
    expect(io.text().stderr).toContain("usage: sage");
    expect(io.text().stderr).not.toContain("--json");
  });

  it("prints health on sage -p and the same object from stats", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-health-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(["-p", "hi", "--passthrough"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { anthropic: adapter as ProviderAdapter },
    });
    expect(code).toBe(0);
    const summary = JSON.parse(io.text().stdout.trim()) as {
      health: {
        tokens: {
          input: number;
          cacheRead: number;
          cacheCreation: number;
          output: number;
          total: number;
        };
        cacheHitRatio: number | null;
      };
    };
    expect(summary.health.tokens).toEqual({
      input: 3,
      cacheRead: 1,
      cacheCreation: 0,
      output: 2,
      total: 6,
    });
    expect(summary.health.cacheHitRatio).toBe(0.25);
    expect(onlyCounts(summary.health)).toBe(true);

    const sessions = readdirSync(path.join(dir, ".sage", "sessions"));
    const logName = sessions.find((name) => name.endsWith(".jsonl"));
    expect(logName).toBeDefined();
    const file = path.join(dir, ".sage", "sessions", logName ?? "");
    const stats = capture();
    const statsCode = await runCli(["stats", file], {
      cwd: dir,
      env: {},
      stdout: stats.stdout,
      stderr: stats.stderr,
    });
    expect(statsCode).toBe(0);
    const printed = stats.text().stdout.trim();
    expect(JSON.parse(printed)).toEqual({
      ...summary.health,
      shadowNone: 0,
      touch: { laterNeeded: 0, precision: null },
      content: { laterNeeded: 0, precision: null },
    });
    expect(printed).not.toContain("You are sage");
    expect(printed).not.toContain(file);
    expect(onlyCounts(JSON.parse(printed))).toBe(true);
  });
});

describe("ladder flag", () => {
  it("parses --ladder and rejects a rung that is not provider/model", () => {
    expect(
      parseArgs([
        "--print",
        "hi",
        "--ladder",
        "openai/gpt-5.4-mini,anthropic/claude-sonnet-5",
      ]),
    ).toMatchObject({
      ok: true,
      ladder: [
        { provider: "openai", model: "gpt-5.4-mini" },
        { provider: "anthropic", model: "claude-sonnet-5" },
      ],
    });
    expect(
      parseArgs(["--print", "hi", "--ladder=anthropic/claude-sonnet-5"]),
    ).toMatchObject({
      ok: true,
      ladder: [{ provider: "anthropic", model: "claude-sonnet-5" }],
    });
    expect(parseArgs(["--print", "hi", "--ladder", "gpt-5.4"]).ok).toBe(false);
    const help = parseArgs(["--help"]);
    if (help.ok) throw new Error("help should not parse");
    expect(help.message).toContain("--ladder");
  });
});

describe("scorer flag", () => {
  it("parses --scorer model and rejects an unknown scorer", () => {
    expect(parseArgs(["--print", "hi", "--scorer", "model"])).toMatchObject({
      ok: true,
      task: "hi",
      scorer: "model",
    });
    expect(parseArgs(["--print", "hi", "--scorer=rule"])).toMatchObject({
      ok: true,
      scorer: "rule",
    });
    expect(parseArgs(["--print", "hi", "--scorer", "jev"])).toMatchObject({
      ok: true,
      scorer: "jev",
    });
    expect(parseArgs(["--print", "hi", "--scorer", "nope"]).ok).toBe(false);
    expect(
      parseArgs(["--print", "hi", "--shadow-scorer", "model"]),
    ).toMatchObject({
      ok: true,
      shadowScorer: "model",
    });
    expect(parseArgs(["--print", "hi", "--shadow-scorer=rule"])).toMatchObject({
      ok: true,
      shadowScorer: "rule",
    });
    expect(parseArgs(["--print", "hi", "--shadow-scorer", "nope"]).ok).toBe(
      false,
    );
    const help = parseArgs(["--help"]);
    if (help.ok) throw new Error("help should not parse");
    expect(help.message).toContain("jev");
    expect(help.message).toContain("--shadow-scorer");
  });

  it("exits 2 when --scorer jev has no TYPESAFE_API_KEY", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-jev-"));
    const io = capture();
    const code = await runCli(["--print", "hi", "--scorer", "jev"], {
      cwd: dir,
      env: {},
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(2);
    expect(io.text().stderr).toContain("TYPESAFE_API_KEY");
  });

  it("exits 2 when --shadow-scorer jev has no TYPESAFE_API_KEY", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-shadow-jev-"));
    const io = capture();
    const code = await runCli(["--print", "hi", "--shadow-scorer", "jev"], {
      cwd: dir,
      env: {},
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(2);
    expect(io.text().stderr).toContain("TYPESAFE_API_KEY");
  });

  it("rejects --provider typesafe because it is not a chat endpoint", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-typesafe-"));
    const io = capture();
    const code = await runCli(["--print", "hi", "--provider", "typesafe"], {
      cwd: dir,
      env: { TYPESAFE_API_KEY: "k" },
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(1);
    expect(io.text().stderr).toContain("not a chat endpoint");
    expect(io.text().stdout).toBe("");
  });
});

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

describe("one-shot sage -p run", () => {
  it("prints one JSON summary and a stderr turn line", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-cli-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(["--print", "rename greet"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { anthropic: adapter as ProviderAdapter },
    });
    const text = io.text();
    expect(code).toBe(0);
    expect(text.stderr).toBe("turn 1 · tokens 4/2 · tool calls 0\n");
    const lines = text.stdout.split("\n").filter((line) => line.length > 0);
    expect(lines).toHaveLength(1);
    expect(JSON.parse(lines[0] ?? "")).toMatchObject({
      outcome: "done",
      turns: 1,
    });
    const sessions = readdirSync(path.join(dir, ".sage", "sessions"));
    expect(sessions.some((name) => name.endsWith(".jsonl"))).toBe(true);
  });

  it("rejects injected adapters that do not serve the selected provider instead of reaching the network", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-cli-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    await expect(
      runCli(["--print", "rename greet"], {
        cwd: dir,
        env: { ANTHROPIC_API_KEY: "test-key" },
        stdout: io.stdout,
        stderr: io.stderr,
        adapters: { openai: adapter as ProviderAdapter },
      }),
    ).rejects.toThrow(
      "Injected adapters do not serve the selected provider: anthropic",
    );
  }, 1000);

  it("accepts -p and a positional task, and exits 1 when the run is not done", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-cli-"));
    const { adapter } = createFaux([
      [
        { type: "block_start", index: 0, blockType: "text" },
        { type: "text_delta", index: 0, text: "nope" },
        { type: "block_end", index: 0 },
        {
          type: "usage",
          usage: {
            inputOther: 3,
            inputCacheRead: 1,
            inputCacheCreation: 0,
            output: 2,
          },
        },
        {
          type: "finish",
          kind: "error",
          error: { message: "nope", cause: "other" },
        },
      ],
    ]);
    const io = capture();
    const code = await runCli(["-p", "fail"], {
      cwd: dir,
      env: { OPENAI_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { openai: adapter as ProviderAdapter },
    });
    expect(code).toBe(1);
    expect(JSON.parse(io.text().stdout.trim())).toMatchObject({
      outcome: "error",
    });

    const positional = capture();
    const { adapter: positionalAdapter } = createFaux([stopScript("ok")]);
    const positionalCode = await runCli(["ship it"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: positional.stdout,
      stderr: positional.stderr,
      adapters: { anthropic: positionalAdapter as ProviderAdapter },
    });
    expect(positionalCode).toBe(0);
  });

  it("uses defaultModel when --model is omitted", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-cli-"));
    const { defaultModel } = await import("@sage/provider");
    const { adapter, requests } = createFaux([stopScript("ok")]);
    const io = capture();
    await runCli(["--print", "hi"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key", OPENAI_API_KEY: "other" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { openai: adapter as ProviderAdapter },
    });
    expect(requests[0]?.model).toEqual(defaultModel("openai"));

    const open = capture();
    const faux = createFaux([stopScript("ok")]);
    await runCli(["--provider", "openai", "--model", "gpt-5-mini", "hi"], {
      cwd: dir,
      env: { OPENAI_API_KEY: "test-key" },
      stdout: open.stdout,
      stderr: open.stderr,
      adapters: { openai: faux.adapter as ProviderAdapter },
    });
    expect(faux.requests[0]?.model).toEqual({
      provider: "openai",
      model: "gpt-5-mini",
    });

    const fallback = capture();
    const fallbackFaux = createFaux([stopScript("ok")]);
    await runCli(["--provider", "openai", "hi"], {
      cwd: dir,
      env: { OPENAI_API_KEY: "test-key" },
      stdout: fallback.stdout,
      stderr: fallback.stderr,
      adapters: { openai: fallbackFaux.adapter as ProviderAdapter },
    });
    expect(fallbackFaux.requests[0]?.model).toEqual(defaultModel("openai"));
  });

  it("starts from the bin and exits 1 without a key", () => {
    const env = { ...process.env };
    delete env.ANTHROPIC_API_KEY;
    delete env.OPENAI_API_KEY;
    const result = spawnSync(
      process.execPath,
      ["apps/sage/bin/sage.mjs", "-p", "hi"],
      { cwd: process.cwd(), env, encoding: "utf8" },
    );
    expect(result.status).toBe(1);
    expect(result.stdout).toBe("");
    expect(result.stderr).toMatch(/API_KEY/);
  });

  it("exits 1 when no API key is set and does not print a summary", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-cli-"));
    const io = capture();
    const code = await runCli(["sage-task"], {
      cwd: dir,
      env: {},
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(1);
    expect(io.text().stdout).toBe("");
    expect(io.text().stderr).toMatch(/API_KEY/);
  });

  it("posts --provider deepseek to the preset URL and exits 2 without a key", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-deepseek-"));
    const calls: { url: string; authorization: string | null }[] = [];
    const disabled = globalThis.fetch;
    globalThis.fetch = (async (input, init) => {
      const headers = new Headers(init?.headers);
      calls.push({
        url: String(input),
        authorization: headers.get("authorization"),
      });
      const sse = [
        'data: {"id":"chatcmpl-s","object":"chat.completion.chunk","model":"deepseek-flash","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}',
        "",
        'data: {"id":"chatcmpl-s","object":"chat.completion.chunk","model":"deepseek-flash","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
        "",
        'data: {"id":"chatcmpl-s","object":"chat.completion.chunk","model":"deepseek-flash","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1,"prompt_cache_hit_tokens":1,"prompt_cache_miss_tokens":2}}',
        "",
        "data: [DONE]",
        "",
      ].join("\n");
      return new Response(sse, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    }) as typeof fetch;
    try {
      const io = capture();
      const code = await runCli(["--provider", "deepseek", "--print", "hi"], {
        cwd: dir,
        env: { DEEPSEEK_API_KEY: "sk-deepseek" },
        stdout: io.stdout,
        stderr: io.stderr,
      });
      expect(code).toBe(0);
      expect(calls[0]?.url).toBe("https://api.deepseek.com/chat/completions");
      expect(calls[0]?.authorization).toBe("Bearer sk-deepseek");
    } finally {
      globalThis.fetch = disabled;
    }

    const missing = capture();
    const missingCode = await runCli(["--provider", "deepseek", "hi"], {
      cwd: dir,
      env: {},
      stdout: missing.stdout,
      stderr: missing.stderr,
    });
    expect(missingCode).toBe(2);
    expect(missing.text().stdout).toBe("");
    expect(missing.text().stderr).toContain("DEEPSEEK_API_KEY");
  });

  it("picks deepseek-flash when only DEEPSEEK_API_KEY is set", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-deepseek-default-"));
    const { adapter, requests } = createFaux([stopScript("ok")]);
    const io = capture();
    const code = await runCli(["-p", "hi"], {
      cwd: dir,
      env: { DEEPSEEK_API_KEY: "sk-deepseek" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { deepseek: adapter as ProviderAdapter },
    });
    expect(code).toBe(0);
    expect(requests[0]?.model).toEqual({
      provider: "deepseek",
      model: "deepseek-flash",
    });

    const explicit = capture();
    const explicitFaux = createFaux([stopScript("ok")]);
    const explicitCode = await runCli(["-p", "hi", "--provider", "anthropic"], {
      cwd: dir,
      env: {
        DEEPSEEK_API_KEY: "sk-deepseek",
        ANTHROPIC_API_KEY: "sk-anthropic",
      },
      stdout: explicit.stdout,
      stderr: explicit.stderr,
      adapters: { anthropic: explicitFaux.adapter as ProviderAdapter },
    });
    expect(explicitCode).toBe(0);
    expect(explicitFaux.requests[0]?.model.provider).toBe("anthropic");
  });

  it("logs reasoning_content and leaves it out of the next request", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-reason-"));
    const fixture = readFileSync(
      new URL(
        "../../../packages/provider/test/fixtures/deepseek-reasoner.sse",
        import.meta.url,
      ),
      "utf8",
    );
    const calls: unknown[] = [];
    const fetchImpl: typeof fetch = async (_input, init) => {
      calls.push(JSON.parse(String(init?.body ?? "null")));
      const sse =
        calls.length === 1
          ? fixture
          : [
              'data: {"id":"chatcmpl-2","object":"chat.completion.chunk","model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}]}',
              "",
              'data: {"id":"chatcmpl-2","object":"chat.completion.chunk","model":"deepseek-v4-pro","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
              "",
              'data: {"id":"chatcmpl-2","object":"chat.completion.chunk","model":"deepseek-v4-pro","choices":[],"usage":{"prompt_tokens":4,"completion_tokens":1,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":4}}',
              "",
              "data: [DONE]",
              "",
            ].join("\n");
      return new Response(sse, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    };
    const adapter = new OpenAIAdapter({
      apiKey: "sk-deepseek",
      baseUrl: "https://api.deepseek.com",
      fetchImpl,
    });
    const { session, store } = createCodingSession({
      cwd: dir,
      adapters: { deepseek: adapter },
      model: { provider: "deepseek", model: "deepseek-v4-pro" },
      sessionId: "reason",
      passthrough: true,
    });
    const summary = await session.run("hi");
    expect(summary.outcome).toBe("done");
    const assistant = store
      .path("reason")
      .filter((entry) => entry.kind === "message")
      .map((entry) => entry.payload)
      .find(
        (payload): payload is { role: string; content: { type: string }[] } =>
          typeof payload === "object" &&
          payload !== null &&
          (payload as { role?: string }).role === "assistant",
      );
    expect(assistant?.content).toContainEqual({
      type: "thinking",
      text: "REASON_STEP",
    });
    expect(assistant?.content[0]).not.toHaveProperty("signature");
    expect(JSON.stringify(calls[1])).not.toContain("REASON_STEP");
    expect(JSON.stringify(calls[1])).not.toContain("reasoning_content");
  });
});

describe("rule scorer on the real tools", () => {
  // Drives the shipped bash and edit tools, so a renamed tool or a changed
  // result line breaks the rule scorer's name table here.
  it("summarizes a passing bash runner at entry and leaves the edit echo full", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-rule-"));
    writeFileSync(path.join(dir, "a.txt"), "hello\n");
    const command = `node -e "process.stdout.write('ok '.repeat(800))"`;
    const { adapter } = createFaux([
      toolScript("bash1", "bash", JSON.stringify({ command })),
      toolScript(
        "edit1",
        "edit",
        JSON.stringify({
          path: "a.txt",
          old_string: "hello",
          new_string: "bye",
        }),
      ),
      stopScript("done"),
    ]);
    const io = capture();
    const code = await runCli(
      ["-p", "run it", "--scorer", "rule", "--scorer-apply", "entry"],
      {
        cwd: dir,
        env: { ANTHROPIC_API_KEY: "test-key" },
        stdout: io.stdout,
        stderr: io.stderr,
        adapters: { anthropic: adapter as ProviderAdapter },
      },
    );
    expect(code).toBe(0);
    expect(readFileSync(path.join(dir, "a.txt"), "utf8")).toBe("bye\n");

    const sessions = readdirSync(path.join(dir, ".sage", "sessions"));
    const logName = sessions.find((name) => name.endsWith(".jsonl"));
    const entries = readFileSync(
      path.join(dir, ".sage", "sessions", logName ?? ""),
      "utf8",
    )
      .split("\n")
      .filter((line) => line.length > 0)
      .map((line) => JSON.parse(line) as { kind?: string; payload?: unknown });
    const last = entries.filter((entry) => entry.kind === "manifest").at(-1)
      ?.payload as { chunks: { id: string; tier: string; reason: string }[] };
    const bash = last.chunks.find((chunk) => chunk.id.endsWith("#bash1"));
    const edit = last.chunks.find((chunk) => chunk.id.endsWith("#edit1"));
    expect(bash).toMatchObject({
      tier: "summary",
      reason: "scorer:entry:summary;scorer:rule:bash-runner",
    });
    expect(edit?.tier).toBe("full");
    expect(edit?.reason).not.toContain("scorer:");
  });
});

describe("read-then-edit run against a faux model", () => {
  it("edits the workspace and logs passthrough manifests", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-e2e-"));
    writeFileSync(
      path.join(dir, "hello.ts"),
      "export function greet(name: string) {\n  return name;\n}\n",
    );
    const { adapter } = createFaux([
      toolScript("read1", "read", JSON.stringify({ path: "hello.ts" })),
      toolScript(
        "edit1",
        "edit",
        JSON.stringify({
          path: "hello.ts",
          old_string: "function greet",
          new_string: "function sayHello",
        }),
      ),
      stopScript("renamed"),
    ]);
    const { session, store } = createCodingSession({
      cwd: dir,
      adapters: { anthropic: adapter as ProviderAdapter },
      model: { provider: "anthropic", model: "claude-test" },
      sessionId: "e2e",
      passthrough: true,
    });
    const summary = await session.run("rename greet to sayHello");
    expect(summary.outcome).toBe("done");
    expect(readFileSync(path.join(dir, "hello.ts"), "utf8")).toContain(
      "function sayHello",
    );
    const logFile = path.join(dir, ".sage", "sessions", "e2e.jsonl");
    expect(readFileSync(logFile, "utf8").length).toBeGreaterThan(0);
    const pathEntries = store.path("e2e");
    expect(pathEntries.map((entry) => entry.kind)).toEqual([
      "message",
      "request_header",
      "manifest",
      "message",
      "message",
      "request_header",
      "manifest",
      "message",
      "message",
      "request_header",
      "manifest",
      "message",
      "terminal",
    ]);
    const manifests = pathEntries.filter((entry) => entry.kind === "manifest");
    expect(manifests.length).toBe(summary.turns);
    for (const entry of manifests) {
      const payload = entry.payload as {
        anomalies: unknown[];
        chunks: { reason: string }[];
      };
      expect(payload.anomalies).toEqual([]);
      expect(payload).not.toHaveProperty("budget");
      expect(payload.chunks.length).toBeGreaterThan(0);
      for (const chunk of payload.chunks)
        expect(chunk.reason).toBe("passthrough");
    }
  });
});

describe("first-party claims in the coding session", () => {
  it("puts AGENTS.md on the epoch head and the ledger on the next turn's tail", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-claims-"));
    writeFileSync(
      path.join(dir, "AGENTS.md"),
      "Workspace rule: prefer small steps.\n",
    );
    const { adapter, requests } = createFaux([
      toolScript(
        "led1",
        "update_ledger",
        JSON.stringify({ goal: "rename greet" }),
      ),
      stopScript("done"),
    ]);
    const { session } = createCodingSession({
      cwd: dir,
      adapters: { anthropic: adapter as ProviderAdapter },
      model: { provider: "anthropic", model: "claude-test" },
      sessionId: "claims-e2e",
    });
    const summary = await session.run("rename greet");
    expect(summary.outcome).toBe("done");
    expect(requests).toHaveLength(2);

    const head = requests[0]?.messages[0];
    expect(head?.role).toBe("user");
    const headText = textBlocks(head?.content ?? []);
    expect(headText).toContain("Workspace rule: prefer small steps.");
    expect(headText).toContain('<claim id="agents-md"');
    expect(headText).not.toContain("rename greet");

    const last = requests[1]?.messages.at(-1);
    expect(last?.role).toBe("user");
    const tail = textBlocks(last?.content ?? []);
    expect(tail).toContain("Goal: rename greet");
    expect(tail).toContain('<claim id="progress-ledger"');
    const result = (last?.content ?? []).find(
      (block) =>
        typeof block === "object" &&
        block !== null &&
        "type" in block &&
        block.type === "tool_result",
    ) as { content?: string } | undefined;
    expect(result?.content).toBe("Ledger updated.");
  });

  it("keeps a passthrough view when --passthrough is set", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-pass-"));
    writeFileSync(
      path.join(dir, "AGENTS.md"),
      "Workspace rule: prefer small steps.\n",
    );
    const { adapter, requests } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(["--print", "hi", "--passthrough"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { anthropic: adapter as ProviderAdapter },
    });
    expect(code).toBe(0);
    const first = JSON.stringify(requests[0]?.messages[0]);
    expect(first).toContain("hi");
    expect(first).not.toContain("Workspace rule");
    const sessions = readdirSync(path.join(dir, ".sage", "sessions"));
    const logName = sessions.find((name) => name.endsWith(".jsonl"));
    expect(logName).toBeDefined();
    const log = readFileSync(
      path.join(dir, ".sage", "sessions", logName ?? ""),
      "utf8",
    );
    // spawn_subtask's schema includes a budget field, so the whole log
    // contains that word. Passthrough still must not put budget on a manifest.
    const entries = log
      .split("\n")
      .filter((line) => line.length > 0)
      .map((line) => JSON.parse(line) as { kind?: string; payload?: unknown });
    for (const entry of entries) {
      expect(entry.kind).not.toBe("view_epoch");
      if (entry.kind === "manifest") {
        expect(entry.payload).not.toHaveProperty("budget");
      }
    }
  });
});

describe("expanding an omitted chunk", () => {
  it("shows the logged original as the expand tool result on the third request", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-expand-"));
    const original = "ORIGINAL_BYTES";
    writeFileSync(path.join(dir, "note.txt"), original);
    const inputJson = JSON.stringify({ path: "note.txt" });
    const usage = {
      inputOther: 3,
      inputCacheRead: 1,
      inputCacheCreation: 0,
      output: 2,
    };
    const reads = ["r1", "r2", "r3", "r4", "r5", "r6", "r7"].flatMap(
      (id, index) => [
        {
          type: "block_start" as const,
          index,
          blockType: "tool_use",
          id,
          name: "read",
        },
        { type: "tool_input_delta" as const, index, partialJson: inputJson },
        { type: "block_end" as const, index },
      ],
    );
    const { adapter, requests } = createFaux([
      [
        ...reads,
        { type: "usage", usage },
        { type: "finish", kind: "tool_use" },
      ],
      () => {
        writeFileSync(path.join(dir, "note.txt"), "CHANGED");
        return (async function* () {
          for (const chunk of toolScript(
            "ex1",
            "expand",
            JSON.stringify({ id: "seq:3#r1" }),
          )) {
            yield chunk;
          }
        })();
      },
      stopScript("done"),
    ]);
    const { session } = createCodingSession({
      cwd: dir,
      adapters: { anthropic: adapter as ProviderAdapter },
      model: { provider: "anthropic", model: "claude-test" },
      sessionId: "expand-e2e",
    });
    const summary = await session.run("read the note");
    expect(summary.outcome).toBe("done");
    expect(requests).toHaveLength(3);
    const third = requests[2];
    const expanded = (third?.messages ?? []).flatMap((message) =>
      message.content.filter(
        (block) =>
          typeof block === "object" &&
          block !== null &&
          "type" in block &&
          block.type === "tool_result" &&
          "toolUseId" in block &&
          block.toolUseId === "ex1",
      ),
    );
    expect(expanded).toEqual([
      {
        type: "tool_result",
        toolUseId: "ex1",
        content: original,
        isError: false,
      },
    ]);
  });
});

describe("routing across two assembled adapters", () => {
  it("keeps the current model when both providers are assembled and no ladder is set", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-route-"));
    const anthropic = routedFaux("anthropic-messages");
    const openai = routedFaux("openai-chat");
    const { createCodingSession: create, defaultRoutePolicy } = await import(
      "../src/coding-session.ts"
    );
    expect(typeof defaultRoutePolicy).toBe("function");
    const { session, store } = create({
      cwd: dir,
      adapters: {
        anthropic: anthropic.adapter,
        openai: openai.adapter,
      },
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      sessionId: "route-both",
    });
    let head = "";
    session.hooks.observe("view_compiled", ({ view }) => {
      if (!head) head = JSON.stringify(view.head);
    });
    const summary = await session.run("x".repeat(200));
    expect(summary.modelChanges).toBe(0);
    expect(openai.requests).toHaveLength(0);
    expect(anthropic.requests).toHaveLength(1);
    expect(anthropic.requests[0]?.model).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
    expect(JSON.stringify(anthropic.requests[0]?.head)).toBe(head);
    const kinds = store.path("route-both").map((entry) => entry.kind);
    expect(kinds).not.toContain("model_change");
    expect(kinds).toContain("view_epoch");
  });

  it("stays on the current model inside the only assembled provider", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-route-one-"));
    const anthropic = routedFaux("anthropic-messages");
    const { session } = createCodingSession({
      cwd: dir,
      adapters: { anthropic: anthropic.adapter },
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      sessionId: "route-one",
    });
    const summary = await session.run("x".repeat(200));
    expect(summary.modelChanges).toBe(0);
    expect(anthropic.requests[0]?.model).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
  });

  it("denies the non-default provider when the new view touches .env or secrets", async () => {
    const { defaultRoutePolicy } = await import("../src/coding-session.ts");
    const { adaptersFromEnv } = await import("../src/auth.ts");
    const policy = defaultRoutePolicy(["anthropic", "openai"], "anthropic");
    const base = {
      epoch: 1,
      current: { provider: "anthropic", model: "claude-sonnet-5" },
      candidates: [],
      decision: {
        rebuild: true,
        oldTokens: 1,
        newTokens: 1,
        remainingTurns: 1,
        readRate: 0.1,
        writeRate: 1.25,
        savingsPerTurn: 0,
        extraCost: 0,
        paybackTurns: 0,
        reason: "immediate",
      },
      budget: 100,
    };
    const open = policy({ ...base, viewChunks: [] });
    expect(open?.model).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
    const closed = policy({
      ...base,
      viewChunks: [
        {
          id: "env",
          tier: "full",
          reason: "kept",
          accesses: [{ key: "/work/.env", mode: "read" }],
        },
      ],
    });
    expect(closed?.model.provider).not.toBe("openai");
    expect(closed?.model).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
    expect(closed?.reason).toContain("sensitive:/work/.env");
    const secrets = policy({
      ...base,
      viewChunks: [
        {
          id: "sec",
          tier: "line",
          reason: "kept",
          accesses: [{ key: "/work/secrets/token", mode: "read" }],
        },
      ],
    });
    expect(secrets?.reason).toContain("sensitive:/work/secrets/token");
    expect(secrets?.model.provider).toBe("anthropic");
    const hidden = policy({
      ...base,
      viewChunks: [
        {
          id: "gone",
          tier: "none",
          reason: "budget:oldest-first",
          accesses: [{ key: ".env", mode: "read" }],
        },
      ],
    });
    expect(hidden?.model).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-5",
    });
    const only = defaultRoutePolicy(["openai"], "openai");
    const stayed = only({
      ...base,
      current: { provider: "openai", model: "gpt-5.4" },
      viewChunks: [
        {
          id: "env",
          tier: "full",
          reason: "kept",
          accesses: [{ key: ".env", mode: "read" }],
        },
      ],
    });
    expect(stayed?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4",
    });
    const built = adaptersFromEnv(
      { ANTHROPIC_API_KEY: "a", OPENAI_API_KEY: "b" },
      NO_LOCAL_PROVIDERS,
    );
    expect(Object.keys(built).sort()).toEqual(["anthropic", "openai"]);
    expect(built.anthropic?.api).toBe("anthropic-messages");
    expect(built.openai?.api).toBe("openai-chat");
    expect(
      Object.keys(adaptersFromEnv({ OPENAI_API_KEY: "b" }, NO_LOCAL_PROVIDERS)),
    ).toEqual(["openai"]);
  });

  it("picks the cheapest explicit ladder rung and still denies a non-home provider", async () => {
    const { defaultRoutePolicy } = await import("../src/coding-session.ts");
    const policy = defaultRoutePolicy(["anthropic", "openai"], "anthropic", [
      { provider: "openai", model: "gpt-5.4-mini" },
      { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
      { provider: "anthropic", model: "claude-sonnet-5" },
    ]);
    const base = {
      epoch: 1,
      current: { provider: "anthropic", model: "claude-sonnet-5" },
      candidates: [],
      decision: {
        rebuild: true,
        oldTokens: 1,
        newTokens: 1,
        remainingTurns: 1,
        readRate: 0.1,
        writeRate: 1.25,
        savingsPerTurn: 0,
        extraCost: 0,
        paybackTurns: 0,
        reason: "immediate",
      },
      budget: 100,
    };
    expect(policy({ ...base, viewChunks: [] })?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4-mini",
    });
    const closed = policy({
      ...base,
      viewChunks: [
        {
          id: "env",
          tier: "full",
          reason: "kept",
          accesses: [{ key: "/work/.env", mode: "read" }],
        },
      ],
    });
    expect(closed?.model).toEqual({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    });
    expect(closed?.reason).toContain("sensitive:/work/.env");

    dir = mkdtempSync(path.join(tmpdir(), "sage-route-ladder-"));
    const anthropic = routedFaux("anthropic-messages");
    const openai = routedFaux("openai-chat");
    const { session } = createCodingSession({
      cwd: dir,
      adapters: {
        anthropic: anthropic.adapter,
        openai: openai.adapter,
      },
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      sessionId: "route-ladder",
      ladder: [
        { provider: "openai", model: "gpt-5.4-mini" },
        { provider: "anthropic", model: "claude-sonnet-5" },
      ],
    });
    const summary = await session.run("x".repeat(200));
    expect(summary.modelChanges).toBe(1);
    expect(openai.requests[0]?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4-mini",
    });
    expect(anthropic.requests).toHaveLength(0);
  });
});

describe("subtasks", () => {
  it("spawns a read-only child that extends the parent request", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-subtask-"));
    const { adapter, requests } = createFaux([
      toolScript(
        "s1",
        "spawn_subtask",
        JSON.stringify({
          query: "find the bug",
          budget: 2000,
          readOnly: true,
        }),
      ),
      stopScript("child-answer"),
      stopScript("parent-done"),
    ]);
    const { session, store } = createCodingSession({
      cwd: dir,
      adapters: { anthropic: adapter as ProviderAdapter },
      model: { provider: "anthropic", model: "claude-test" },
      sessionId: "sub-e2e",
    });
    const summary = await session.run("look around");
    expect(summary.outcome).toBe("done");
    const parentReq = requests[0];
    const childReq = requests[1];
    if (!parentReq || !childReq) throw new Error("missing requests");
    expect(childReq.head).toEqual(parentReq.head);
    expect(childReq.messages.slice(0, parentReq.messages.length)).toEqual(
      parentReq.messages,
    );
    const match = /Subtask ([0-9a-f-]+) done/.exec(
      JSON.stringify(store.all("sub-e2e")),
    );
    const childId = match?.[1];
    if (!childId) throw new Error("missing child session");
    expect(store.all(childId)[0]?.kind).toBe("subtask");
    expect(JSON.stringify(store.all(childId))).not.toContain("look around");
  });
});

function routedFaux(api: "anthropic-messages" | "openai-chat") {
  const { adapter, requests } = createFaux([stopScript("done")]);
  return {
    requests,
    adapter: {
      api,
      prepare(model: { provider: string; model: string }) {
        return {
          model,
          contextWindow: 10,
          pricing: null,
          adapterDefaults: { maxTokens: 32 },
        };
      },
      stream: adapter.stream.bind(adapter),
    },
  };
}

function onlyCounts(value: unknown): boolean {
  if (value === null) return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    return Object.values(value).every(onlyCounts);
  }
  return false;
}

function textBlocks(content: unknown[]): string {
  return content
    .map((block) => {
      if (
        typeof block === "object" &&
        block !== null &&
        "type" in block &&
        block.type === "text" &&
        "text" in block &&
        typeof block.text === "string"
      ) {
        return block.text;
      }
      return "";
    })
    .join("\n");
}
