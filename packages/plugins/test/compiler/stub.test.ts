import { MemoryLogStore, type ViewContext } from "@sage/core";
import { expect, it } from "vitest";
import { defaultTokenCount } from "../../src/compiler/budget.ts";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { renderStub, summarizeInput } from "../../src/compiler/stub.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };

function chunk(overrides: Partial<Chunk> & Pick<Chunk, "tokens">): Chunk {
  return {
    id: "seq:3#b1",
    kind: "tool",
    messageIndex: 0,
    toolUseId: "b1",
    toolName: "bash",
    input: { cmd: "ls" },
    accesses: [],
    ...overrides,
  };
}

it("renders an omitted tool chunk as a one-line stub", () => {
  expect(summarizeInput({ cmd: "ls" })).toBe('{"cmd":"ls"}');
  expect(summarizeInput("a\n\tb  c")).toBe("a b c");
  expect(summarizeInput(undefined)).toBe("");
  const long = "y".repeat(90);
  expect(summarizeInput(long)).toBe(`${"y".repeat(79)}…`);
  expect(summarizeInput("z".repeat(80))).toBe("z".repeat(80));

  expect(renderStub(chunk({ tokens: 4100 }))).toBe(
    '[#seq:3#b1 · bash · {"cmd":"ls"} · 4.1k tokens · omitted · expand("seq:3#b1") to retrieve]',
  );
  expect(renderStub(chunk({ tokens: 1499 }))).toBe(
    '[#seq:3#b1 · bash · {"cmd":"ls"} · 1.5k tokens · omitted · expand("seq:3#b1") to retrieve]',
  );
});

it("records the stub's token count on the manifest and keeps originalTokens", async () => {
  const rawInput = '{"cmd":"ls"}';
  const result = "boom";
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "b1",
          name: "bash",
          input: { cmd: "ls" },
          rawInput,
        },
      ],
    },
  });
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "b1",
          content: result,
          isError: true,
        },
      ],
    },
  });
  const ctx: ViewContext = {
    sessionId: "s",
    turn: 1,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 8000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [],
  };
  const view = await contextCompiler({
    budget: 0,
    claims: [],
    keepRecent: 0,
  }).view(ctx);

  const stub =
    '[#seq:0#b1 · bash · {"cmd":"ls"} · 0.0k tokens · omitted · expand("seq:0#b1") to retrieve]';
  const shown = view.messages.flatMap((message) =>
    message.content.filter(
      (block) => block.type === "tool_result" && block.toolUseId === "b1",
    ),
  );
  expect(shown).toEqual([
    {
      type: "tool_result",
      toolUseId: "b1",
      content: stub,
      isError: true,
    },
  ]);
  expect(JSON.stringify(view.messages)).not.toContain("[omitted]");
  expect(view.manifest.chunks).toEqual([
    expect.objectContaining({
      id: "seq:0#b1",
      tier: "none",
      tokens: defaultTokenCount(stub),
      originalTokens: defaultTokenCount(rawInput) + defaultTokenCount(result),
    }),
  ]);
});
