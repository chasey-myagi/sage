import { assertPaired, type HarnessTool } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { canSplitAfter, indexChunks } from "../../src/compiler/chunks.ts";

function countExact(table: Record<string, number>) {
  return (text: string) => {
    const tokens = table[text];
    if (tokens === undefined) {
      throw new Error(`no token count for ${JSON.stringify(text)}`);
    }
    return tokens;
  };
}

function declared(
  name: string,
  mode: "read" | "write" | "search",
): HarnessTool {
  return {
    def: {
      name,
      description: name,
      inputSchema: { type: "object" },
    },
    isReadOnly: mode !== "write",
    isDestructive: mode === "write",
    isOpenWorld: false,
    accesses(input: unknown) {
      if (typeof input !== "object" || input === null || !("path" in input)) {
        return [];
      }
      const key = input.path;
      if (typeof key !== "string" || key.length === 0) return [];
      return [{ key, mode }];
    },
    async execute() {
      return { content: "", isError: false };
    },
  };
}

function text(role: "user" | "assistant", body: string): Message {
  return { role, content: [{ type: "text", text: body }] };
}

function call(
  id: string,
  name: string,
  input: unknown,
  rawInput: string,
): Message {
  return {
    role: "assistant",
    content: [{ type: "tool_use", id, name, input, rawInput }],
  };
}

function result(toolUseId: string, content: string, isError = false): Message {
  return {
    role: "user",
    content: [{ type: "tool_result", toolUseId, content, isError }],
  };
}

it("indexes two tool calls in one assistant message as two chunks and prose as seq chunks", () => {
  const rawA = '{"path":"/repo/a.ts"}';
  const rawB = '{"path":"/repo/b.ts"}';
  const inputA = { path: "/repo/a.ts" };
  const inputB = { path: "/repo/b.ts" };
  const messages: Message[] = [
    text("user", "hello"),
    text("assistant", "plan"),
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "A",
          name: "read",
          input: inputA,
          rawInput: rawA,
        },
        {
          type: "tool_use",
          id: "B",
          name: "read",
          input: inputB,
          rawInput: rawB,
        },
      ],
    },
    {
      role: "user",
      content: [
        { type: "tool_result", toolUseId: "B", content: "bee", isError: false },
        {
          type: "tool_result",
          toolUseId: "A",
          content: "alpha",
          isError: true,
        },
      ],
    },
  ];
  const chunks = indexChunks(
    messages,
    [1, 2, 7, 8],
    [declared("read", "read")],
    countExact({
      hello: 5,
      plan: 4,
      [rawA]: 10,
      [rawB]: 11,
      alpha: 3,
      bee: 7,
    }),
  );

  expect(chunks.map((chunk) => chunk.id)).toEqual([
    "seq:1",
    "seq:2",
    "seq:7#A",
    "seq:7#B",
  ]);
  expect(chunks[0]).toEqual({
    id: "seq:1",
    kind: "user",
    messageIndex: 0,
    tokens: 5,
    accesses: [],
  });
  expect(chunks[1]).toEqual({
    id: "seq:2",
    kind: "assistant",
    messageIndex: 1,
    tokens: 4,
    accesses: [],
  });
  expect(chunks[2]).toEqual({
    id: "seq:7#A",
    kind: "tool",
    messageIndex: 2,
    toolUseId: "A",
    toolName: "read",
    input: inputA,
    isError: true,
    tokens: 13,
    accesses: [{ key: "/repo/a.ts", mode: "read" }],
  });
  expect(chunks[3]).toEqual({
    id: "seq:7#B",
    kind: "tool",
    messageIndex: 2,
    toolUseId: "B",
    toolName: "read",
    input: inputB,
    isError: false,
    tokens: 18,
    accesses: [{ key: "/repo/b.ts", mode: "read" }],
  });
  assertPaired(messages);
});

it("keeps prose beside a tool call as its own chunk and skips a result-only user message", () => {
  const raw = '{"path":"/repo/a.ts"}';
  const messages: Message[] = [
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "A",
          name: "read",
          input: { path: "/repo/a.ts" },
          rawInput: raw,
        },
        { type: "text", text: "see" },
      ],
    },
    result("A", "body"),
  ];
  const chunks = indexChunks(
    messages,
    [4, 5],
    [declared("read", "read")],
    countExact({ see: 1, [raw]: 2, body: 4 }),
  );
  expect(
    chunks.map((chunk) => ({
      id: chunk.id,
      kind: chunk.kind,
      tokens: chunk.tokens,
    })),
  ).toEqual([
    { id: "seq:4", kind: "assistant", tokens: 1 },
    { id: "seq:4#A", kind: "tool", tokens: 6 },
  ]);
});

it.each([
  {
    name: "after a user message",
    messages: [text("user", "hi"), text("assistant", "ok")] as const,
    index: 0,
    expected: false,
  },
  {
    name: "after an assistant message that contains tool_use",
    messages: [
      call("A", "read", { path: "/repo/a.ts" }, "{}"),
      result("A", "body"),
    ] as const,
    index: 0,
    expected: false,
  },
  {
    name: "when the next message is a user message containing tool_result",
    messages: [text("assistant", "note"), result("A", "body")] as const,
    index: 0,
    expected: false,
  },
  {
    name: "after assistant prose when the next message is user prose",
    messages: [text("assistant", "done"), text("user", "next")] as const,
    index: 0,
    expected: true,
  },
])("canSplitAfter is $name → $expected", ({ messages, index, expected }) => {
  expect(canSplitAfter(messages, index)).toBe(expected);
});
