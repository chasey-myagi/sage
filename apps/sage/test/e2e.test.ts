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
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, describe, expect, it } from "vitest";
import {
  createFaux,
  stopScript,
  toolScript,
} from "../../../packages/core/test/support/faux.ts";
import { createCodingSession, parseArgs, runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
});

describe("health output", () => {
  it("parses --json", () => {
    expect(parseArgs(["--print", "hi", "--json"])).toMatchObject({
      ok: true,
      task: "hi",
      json: true,
    });
    expect(parseArgs(["-p", "--json", "hi"])).toMatchObject({
      ok: true,
      task: "hi",
      json: true,
    });
  });

  it("prints health on sage -p --json and the same object from stats", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-health-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(["-p", "--json", "hi", "--passthrough"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapter: adapter as ProviderAdapter,
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
    expect(JSON.parse(printed)).toEqual(summary.health);
    expect(printed).not.toContain("You are sage");
    expect(printed).not.toContain(file);
    expect(onlyCounts(JSON.parse(printed))).toBe(true);
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
    expect(parseArgs(["--print", "hi", "--scorer", "nope"]).ok).toBe(false);
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

describe("scenario 7 one-shot cli", () => {
  it("prints one JSON summary and a stderr turn line", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-cli-"));
    const { adapter } = createFaux([stopScript("done")]);
    const io = capture();
    const code = await runCli(["--print", "rename greet"], {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapter: adapter as ProviderAdapter,
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
        { type: "finish", kind: "error", error: { message: "nope" } },
      ],
    ]);
    const io = capture();
    const code = await runCli(["-p", "fail"], {
      cwd: dir,
      env: { OPENAI_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapter: adapter as ProviderAdapter,
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
      adapter: positionalAdapter as ProviderAdapter,
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
      adapter: adapter as ProviderAdapter,
    });
    expect(requests[0]?.model).toEqual(defaultModel("anthropic"));

    const open = capture();
    const faux = createFaux([stopScript("ok")]);
    await runCli(["--provider", "openai", "--model", "gpt-5-mini", "hi"], {
      cwd: dir,
      env: { OPENAI_API_KEY: "test-key" },
      stdout: open.stdout,
      stderr: open.stderr,
      adapter: faux.adapter as ProviderAdapter,
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
      adapter: fallbackFaux.adapter as ProviderAdapter,
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
});

describe("scenario 8 faux read then edit", () => {
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
      adapter: adapter as ProviderAdapter,
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
      adapter: adapter as ProviderAdapter,
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
      adapter: adapter as ProviderAdapter,
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

describe("scenario 6 expand an omitted chunk", () => {
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
      adapter: adapter as ProviderAdapter,
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

describe("scenario 9 two adapters", () => {
  it("switches to the cheapest model when both providers are assembled", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-route-"));
    const anthropic = routedFaux("anthropic-messages");
    const openai = routedFaux("openai-chat");
    const { createCodingSession: create, defaultRoutePolicy } = await import(
      "../src/main.ts"
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
    expect(summary.modelChanges).toBe(1);
    expect(openai.requests).toHaveLength(1);
    expect(openai.requests[0]?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4-mini",
    });
    expect(anthropic.requests).toHaveLength(0);
    expect(JSON.stringify(openai.requests[0]?.head)).toBe(head);
    const kinds = store.path("route-both").map((entry) => entry.kind);
    const changeAt = kinds.indexOf("model_change");
    expect(changeAt).toBeGreaterThan(-1);
    expect(kinds[changeAt + 1]).toBe("view_epoch");
  });

  it("stays inside the only assembled provider", async () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-route-one-"));
    const anthropic = routedFaux("anthropic-messages");
    const { session } = createCodingSession({
      cwd: dir,
      adapters: { anthropic: anthropic.adapter },
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      sessionId: "route-one",
    });
    const summary = await session.run("x".repeat(200));
    expect(summary.modelChanges).toBe(1);
    expect(anthropic.requests[0]?.model).toEqual({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    });
  });

  it("denies the non-default provider when the new view touches .env or secrets", async () => {
    const { defaultRoutePolicy, adaptersFromEnv } = await import(
      "../src/main.ts"
    );
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
    expect(open?.model).toEqual({ provider: "openai", model: "gpt-5.4-mini" });
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
      model: "claude-haiku-4-5-20251001",
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
    expect(hidden?.model.provider).toBe("openai");
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
      model: "gpt-5.4-mini",
    });
    const built = adaptersFromEnv({
      ANTHROPIC_API_KEY: "a",
      OPENAI_API_KEY: "b",
    });
    expect(Object.keys(built).sort()).toEqual(["anthropic", "openai"]);
    expect(built.anthropic?.api).toBe("anthropic-messages");
    expect(built.openai?.api).toBe("openai-chat");
    expect(Object.keys(adaptersFromEnv({ OPENAI_API_KEY: "b" }))).toEqual([
      "openai",
    ]);
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
      adapter: adapter as ProviderAdapter,
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
