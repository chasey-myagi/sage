import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  AgentSession,
  MemoryLogStore,
  passthroughView,
  type ViewContext,
} from "@sage/core";
import { afterEach, expect, it } from "vitest";
import {
  createFaux,
  MODEL,
  SYSTEM,
  stopScript,
  toolScript,
} from "../../../core/test/support/faux.ts";
import { createTools } from "../../src/index.ts";

const ctx = { sessionId: "s", toolUseId: "tool-79" };
let dir = "";
const outsidePaths: string[] = [];

function freshDir(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-issue-79-"));
  return dir;
}

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  for (const target of outsidePaths.splice(0)) {
    rmSync(target, { recursive: true, force: true });
  }
});

function outsideFile(): string {
  const target = path.join(tmpdir(), `sage-issue-79-outside-${Date.now()}.txt`);
  outsidePaths.push(target);
  writeFileSync(target, "secret-token");
  return target;
}

it("returns an isError tool_result for an outside read and does not throw", async () => {
  const cwd = freshDir();
  const outside = outsideFile();
  const tools = createTools({ cwd });
  const read = tools.find((tool) => tool.def.name === "read");
  if (!read) throw new Error("missing read");

  expect(() => read.accesses({ path: outside })).not.toThrow();
  const result = await read.execute({ path: outside }, ctx);
  expect(result.isError).toBe(true);
  expect(result.content).toBe(
    `[error] Path is outside the workspace: ${outside}`,
  );
  expect(result.content).not.toContain("secret-token");

  const store = new MemoryLogStore();
  const sessionId = "read-outside";
  const { adapter } = createFaux([
    toolScript("r1", "read", JSON.stringify({ path: outside })),
    stopScript("done"),
  ]);
  const session = new AgentSession({
    store,
    sessionId,
    adapter,
    model: MODEL,
    system: SYSTEM,
    tools,
    viewProvider: {
      async view(viewCtx: ViewContext) {
        for (const entry of viewCtx.path) {
          if (entry.kind !== "message") continue;
          const payload = entry.payload as { content?: unknown[] };
          for (const block of payload.content ?? []) {
            if (typeof block !== "object" || block === null) continue;
            const call = block as {
              type?: string;
              name?: string;
              input?: unknown;
            };
            if (call.type !== "tool_use" || typeof call.name !== "string") {
              continue;
            }
            const tool = viewCtx.tools.find(
              (item) => item.def.name === call.name,
            );
            tool?.accesses(call.input);
          }
        }
        return passthroughView.view(viewCtx);
      },
    },
  });

  const summary = await session.run("read it");
  expect(summary.outcome).not.toBe("error");
  const logged = store.path(sessionId).flatMap((entry) => {
    if (entry.kind !== "message") return [];
    const payload = entry.payload as { content?: unknown[] };
    return payload.content ?? [];
  });
  expect(logged).toContainEqual({
    type: "tool_result",
    toolUseId: "r1",
    content: `[error] Path is outside the workspace: ${outside}`,
    isError: true,
  });
});

it("returns fail instead of throwing when ls, grep, find, edit, or write escapes", async () => {
  const cwd = freshDir();
  const outside = outsideFile();
  const tools = createTools({ cwd });
  const cases = [
    { name: "ls", input: { path: outside } },
    { name: "grep", input: { pattern: "secret-token", path: outside } },
    { name: "find", input: { glob: "**/*", path: outside } },
    {
      name: "edit",
      input: { path: outside, old_string: "secret", new_string: "x" },
    },
    { name: "write", input: { path: outside, content: "nope" } },
  ];
  for (const item of cases) {
    const tool = tools.find((candidate) => candidate.def.name === item.name);
    if (!tool) throw new Error(`missing ${item.name}`);
    expect(() => tool.accesses(item.input), item.name).not.toThrow();
    const result = await tool.execute(item.input, ctx);
    expect(result.isError, item.name).toBe(true);
    expect(result.content, item.name).toBe(
      `[error] Path is outside the workspace: ${outside}`,
    );
    expect(result.content, item.name).not.toContain("secret-token");
  }
});
