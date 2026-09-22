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
import { createCodingSession, runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
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
    expect(log).not.toContain('"budget"');
    expect(log).not.toContain("view_epoch");
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
