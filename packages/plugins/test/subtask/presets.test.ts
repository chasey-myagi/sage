import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { MemoryLogStore } from "@sage/core";
import { FAUX_UNIT_USAGE, stopScript } from "@sage/core/testing";
import type { Request } from "@sage/provider/contract";
import { afterEach, expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { subtaskTools } from "../../src/index.ts";
import { loadPresets } from "../../src/subtask/presets.ts";

const SYSTEM = "You are sage.";

let dirs: string[] = [];

afterEach(() => {
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
  dirs = [];
});

function tempDir(): string {
  const dir = mkdtempSync(path.join(tmpdir(), "sage-preset-"));
  dirs.push(dir);
  return dir;
}

function writePreset(root: string, name: string, body: string) {
  const dir = path.join(root, ".sage", "subtasks");
  mkdirSync(dir, { recursive: true });
  writeFileSync(path.join(dir, `${name}.md`), body);
}

function user(text: string) {
  return {
    role: "user" as const,
    content: [{ type: "text" as const, text }],
  };
}

function seed(store: MemoryLogStore, sessionId: string) {
  store.append(sessionId, { kind: "message", payload: user("seed") });
  store.append(sessionId, {
    kind: "request_header",
    payload: {
      provider: "anthropic",
      model: "claude-test",
      systemPromptHash: "h",
      toolsHash: "t",
      sampling: {},
      adapterDefaults: {},
    },
  });
  store.append(sessionId, {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      claims: [],
      chunks: [{ id: "seq:0", tier: "full", reason: "kept", tokens: 1 }],
      anomalies: [],
    },
  });
}

function queryOf(text: string): string {
  const match =
    /<claim id="subtask-query" tier="full">\n([\s\S]*?)\n<\/claim>/.exec(text);
  return match?.[1] ?? "";
}

function messageText(request: Request): string {
  let text = "";
  for (const message of request.messages) {
    for (const block of message.content) {
      if (block.type === "text") text += block.text;
    }
  }
  return text;
}

it("renders a project preset over the user preset and lets explicit fields win", async () => {
  const cwd = tempDir();
  const home = tempDir();
  writePreset(
    home,
    "review",
    [
      "---",
      "budget: 1000",
      "readOnly: false",
      "---",
      "User {{args.path}}",
    ].join("\n"),
  );
  writePreset(
    home,
    "lint",
    [
      "---",
      "budget: 100",
      "readOnly: true",
      "---",
      "Lint {{args.target}}",
    ].join("\n"),
  );
  writePreset(
    cwd,
    "review",
    [
      "---",
      "name: review",
      "budget: 2000",
      "readOnly: true",
      "model: anthropic/claude-haiku-4-5-20251001",
      "---",
      "Project {{args.path}}",
    ].join("\n"),
  );
  writePreset(
    cwd,
    "notes",
    ["---", "name: other", "---", "Notes {{args.topic}}"].join("\n"),
  );

  const presets = loadPresets({ cwd, home });
  const review = presets.get("review");
  expect(review?.query).toBe("Project {{args.path}}");
  expect(review?.budget).toBe(2000);
  expect(review?.readOnly).toBe(true);
  expect(review?.model).toEqual({
    provider: "anthropic",
    model: "claude-haiku-4-5-20251001",
  });
  expect(presets.get("lint")?.query).toBe("Lint {{args.target}}");
  expect(presets.get("notes")?.name).toBe("notes");
  expect(presets.has("other")).toBe(false);

  const store = new MemoryLogStore();
  seed(store, "parent");
  const queries: string[] = [];
  const adapter = {
    api: "anthropic-messages" as const,
    prepare(model: { provider: string; model: string }) {
      return {
        model,
        contextWindow: 8000,
        pricing: null,
        adapterDefaults: {},
      };
    },
    stream(request: Request) {
      const text = messageText(request);
      if (text.includes("Begin the subtask")) queries.push(queryOf(text));
      return (async function* () {
        for (const chunk of stopScript("reviewed", FAUX_UNIT_USAGE))
          yield chunk;
      })();
    },
  };
  const tools: HarnessTool[] = [];
  const { spawn } = subtaskTools({
    store,
    adapters: { anthropic: adapter },
    tools,
    system: SYSTEM,
    cwd,
    home,
    makeCompiler: ({ budget, query }) =>
      contextCompiler({
        budget,
        claims: [],
        subtask: { query },
        expectedTurns: 1,
      }),
  });
  const result = await spawn.execute(
    { preset: "review", args: { path: "src/a.ts" }, budget: 50 },
    { sessionId: "parent", toolUseId: "preset" },
  );
  expect(result.isError).toBe(false);
  expect(queries).toEqual(["Project src/a.ts"]);
  const childId = result.details?.sessionId;
  expect(typeof childId).toBe("string");
  const subtask = store
    .path(String(childId))
    .find((entry) => entry.kind === "subtask");
  expect(subtask?.payload).toMatchObject({
    query: "Project src/a.ts",
    budget: 50,
    readOnly: true,
  });
  const header = store
    .path(String(childId))
    .find((entry) => entry.kind === "request_header");
  expect(header?.payload).toMatchObject({
    provider: "anthropic",
    model: "claude-haiku-4-5-20251001",
  });

  const overridden = await spawn.execute(
    {
      preset: "review",
      query: "explicit question",
      args: { path: "src/a.ts" },
      readOnly: false,
    },
    { sessionId: "parent", toolUseId: "override" },
  );
  expect(overridden.isError).toBe(false);
  const overrideId = overridden.details?.sessionId;
  const overrideEntry = store
    .path(String(overrideId))
    .find((entry) => entry.kind === "subtask");
  expect(overrideEntry?.payload).toMatchObject({
    query: "explicit question",
    budget: 2000,
    readOnly: false,
  });

  const missing = await spawn.execute(
    { preset: "missing" },
    { sessionId: "parent", toolUseId: "missing" },
  );
  expect(missing.isError).toBe(true);
  expect(missing.content).toContain("missing");
});
