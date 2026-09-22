import type { HarnessTool } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { expect, it } from "vitest";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { indexChunks } from "../../src/compiler/chunks.ts";
import { eliminate } from "../../src/compiler/eliminate.ts";

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

const tools = [
  declared("read", "read"),
  declared("write", "write"),
  declared("search", "search"),
  declared("bash", "read"),
];

function countAll(text: string): number {
  return text.length;
}

function toolMessage(
  id: string,
  name: string,
  path: string,
  output: string,
  isError = false,
): Message[] {
  const input = { path };
  const rawInput = JSON.stringify(input);
  return [
    {
      role: "assistant",
      content: [{ type: "tool_use", id, name, input, rawInput }],
    },
    {
      role: "user",
      content: [
        { type: "tool_result", toolUseId: id, content: output, isError },
      ],
    },
  ];
}

function indexToolHistory(
  pairs: {
    seq: number;
    id: string;
    name: string;
    path: string;
    output: string;
    isError?: boolean;
  }[],
): Chunk[] {
  const messages: Message[] = [];
  const seqs: number[] = [];
  for (const pair of pairs) {
    const [assistant, user] = toolMessage(
      pair.id,
      pair.name,
      pair.path,
      pair.output,
      pair.isError ?? false,
    );
    if (!assistant || !user) throw new Error("pair");
    messages.push(assistant, user);
    seqs.push(pair.seq, pair.seq + 1);
  }
  return indexChunks(messages, seqs, tools, countAll);
}

function piece(
  id: string,
  kind: Chunk["kind"],
  tokens: number,
  extra?: Partial<Chunk>,
): Chunk {
  return {
    id,
    kind,
    messageIndex: 0,
    tokens,
    accesses: [],
    ...extra,
  };
}

it("drops an earlier read of the same key when a later read compiles at turn 37", () => {
  const chunks = indexToolHistory([
    { seq: 12, id: "r12", name: "read", path: "/repo/a.ts", output: "old" },
    { seq: 13, id: "o1", name: "read", path: "/repo/o1.ts", output: "1" },
    { seq: 14, id: "o2", name: "read", path: "/repo/o2.ts", output: "2" },
    { seq: 15, id: "o3", name: "read", path: "/repo/o3.ts", output: "3" },
    { seq: 16, id: "o4", name: "read", path: "/repo/o4.ts", output: "4" },
    { seq: 17, id: "o5", name: "read", path: "/repo/o5.ts", output: "5" },
    { seq: 31, id: "r31", name: "read", path: "/repo/a.ts", output: "new" },
  ]);
  const { decisions } = eliminate(chunks, {
    keepRecent: 6,
    remainingBudget: 1_000_000,
  });
  expect(decisions.find((decision) => decision.id === "seq:12#r12")).toEqual({
    id: "seq:12#r12",
    tier: "none",
    reason: "superseded_by:seq:31#r31",
  });
  expect(decisions.find((decision) => decision.id === "seq:31#r31")).toEqual({
    id: "seq:31#r31",
    tier: "full",
    reason: "recent",
  });
  expect(chunks.find((chunk) => chunk.id === "seq:12#r12")?.accesses).toEqual([
    { key: "/repo/a.ts", mode: "read" },
  ]);
  expect(chunks.find((chunk) => chunk.id === "seq:31#r31")?.accesses).toEqual([
    { key: "/repo/a.ts", mode: "read" },
  ]);
});

it("covers a read with a later read, write, or search, and a write only with a later write", () => {
  const cases: {
    earlier: string;
    later: string;
    path: string;
    superseded: boolean;
  }[] = [
    { earlier: "read", later: "write", path: "/repo/a.ts", superseded: true },
    { earlier: "read", later: "search", path: "/repo/a.ts", superseded: true },
    { earlier: "write", later: "read", path: "/repo/a.ts", superseded: false },
    {
      earlier: "write",
      later: "search",
      path: "/repo/a.ts",
      superseded: false,
    },
    { earlier: "write", later: "write", path: "/repo/a.ts", superseded: true },
    {
      earlier: "read",
      later: "read",
      path: "/repo/other.ts",
      superseded: false,
    },
  ];
  for (const spec of cases) {
    const chunks = indexToolHistory([
      {
        seq: 1,
        id: "earlier",
        name: spec.earlier,
        path: "/repo/a.ts",
        output: "old",
      },
      {
        seq: 3,
        id: "later",
        name: spec.later,
        path: spec.path,
        output: "new",
      },
    ]);
    const { decisions } = eliminate(chunks, {
      keepRecent: 0,
      remainingBudget: 1_000_000,
    });
    const earlier = decisions.find(
      (decision) => decision.id === "seq:1#earlier",
    );
    if (spec.superseded) {
      expect(earlier, `${spec.earlier} then ${spec.later}`).toEqual({
        id: "seq:1#earlier",
        tier: "none",
        reason: "superseded_by:seq:3#later",
      });
    } else {
      expect(earlier, `${spec.earlier} then ${spec.later}`).toEqual({
        id: "seq:1#earlier",
        tier: "full",
        reason: "kept",
      });
    }
  }
});

it("does not let an isError access supersede an earlier chunk", () => {
  const chunks = indexToolHistory([
    { seq: 1, id: "ok", name: "read", path: "/repo/a.ts", output: "body" },
    {
      seq: 3,
      id: "bad",
      name: "write",
      path: "/repo/a.ts",
      output: "nope",
      isError: true,
    },
  ]);
  const { decisions } = eliminate(chunks, {
    keepRecent: 0,
    remainingBudget: 1_000_000,
  });
  expect(decisions[0]).toEqual({
    id: "seq:1#ok",
    tier: "full",
    reason: "kept",
  });
});

it("marks the failed attempt retry_intermediate when the next tool chunk repeats the name and input", () => {
  const input = { cmd: "ls", cwd: "/repo" };
  const raw = JSON.stringify(input);
  const messages: Message[] = [
    {
      role: "assistant",
      content: [
        { type: "tool_use", id: "b1", name: "bash", input, rawInput: raw },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "b1",
          content: "boom",
          isError: true,
        },
      ],
    },
    { role: "user", content: [{ type: "text", text: "again" }] },
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "b2",
          name: "bash",
          input: { cwd: "/repo", cmd: "ls" },
          rawInput: raw,
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "b2",
          content: "files",
          isError: false,
        },
      ],
    },
  ];
  const chunks = indexChunks(messages, [1, 2, 3, 4, 5], tools, countAll);
  const { decisions } = eliminate(chunks, {
    keepRecent: 0,
    remainingBudget: 1_000_000,
  });
  expect(decisions.find((decision) => decision.id === "seq:1#b1")).toEqual({
    id: "seq:1#b1",
    tier: "none",
    reason: "retry_intermediate",
  });
  expect(decisions.find((decision) => decision.id === "seq:4#b2")).toEqual({
    id: "seq:4#b2",
    tier: "full",
    reason: "kept",
  });
  expect(decisions.find((decision) => decision.id === "seq:3")).toEqual({
    id: "seq:3",
    tier: "full",
    reason: "kept",
  });
});

it("does not treat a different input or a successful call as a retry", () => {
  const messages: Message[] = [
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "b1",
          name: "bash",
          input: { cmd: "ls", env: { A: "1" } },
          rawInput: '{"cmd":"ls"}',
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "b1",
          content: "boom",
          isError: true,
        },
      ],
    },
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "b2",
          name: "bash",
          input: { env: { A: "1" }, cmd: "pwd" },
          rawInput: '{"cmd":"pwd"}',
        },
      ],
    },
    {
      role: "user",
      content: [
        { type: "tool_result", toolUseId: "b2", content: "no", isError: true },
      ],
    },
  ];
  const chunks = indexChunks(messages, [1, 2, 3, 4], tools, countAll);
  const { decisions } = eliminate(chunks, {
    keepRecent: 0,
    remainingBudget: 1_000_000,
  });
  expect(decisions.map((decision) => decision.reason)).toEqual([
    "kept",
    "kept",
  ]);
});

it("keeps the last keepRecent tool chunks at full even when an earlier rule would drop them", () => {
  const chunks = indexToolHistory([
    {
      seq: 1,
      id: "r1",
      name: "read",
      path: "/repo/a.ts",
      output: "1",
      isError: true,
    },
    { seq: 3, id: "r2", name: "read", path: "/repo/a.ts", output: "2" },
  ]);
  const { decisions } = eliminate(chunks, {
    keepRecent: 6,
    remainingBudget: 1_000_000,
  });
  expect(decisions.map((decision) => decision.reason)).toEqual([
    "recent",
    "recent",
  ]);
});

it("counts only tool chunks toward the recent window", () => {
  const messages: Message[] = [
    {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "old",
          name: "read",
          input: { path: "/repo/a.ts" },
          rawInput: "{}",
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "old",
          content: "old",
          isError: false,
        },
      ],
    },
  ];
  const seqs = [1, 2];
  for (let index = 0; index < 8; index += 1) {
    messages.push({
      role: "user",
      content: [{ type: "text", text: `note-${index}` }],
    });
    seqs.push(10 + index);
  }
  messages.push({
    role: "assistant",
    content: [
      {
        type: "tool_use",
        id: "new",
        name: "read",
        input: { path: "/repo/a.ts" },
        rawInput: "{}",
      },
    ],
  });
  messages.push({
    role: "user",
    content: [
      { type: "tool_result", toolUseId: "new", content: "new", isError: false },
    ],
  });
  seqs.push(30, 31);
  const chunks = indexChunks(messages, seqs, tools, countAll);
  const { decisions } = eliminate(chunks, {
    keepRecent: 6,
    remainingBudget: 1_000_000,
  });
  expect(
    decisions.find((decision) => decision.id === "seq:1#old")?.reason,
  ).toBe("recent");
});

it("demotes the oldest non-recent tool chunks until the kept total fits a 10000 remainder", () => {
  const chunks: Chunk[] = [
    piece("seq:1", "user", 1000),
    piece("seq:2", "assistant", 1000),
    ...[0, 1, 2, 3, 4].map((index) =>
      piece(`old-${index}`, "tool", 5000, {
        toolUseId: `o${index}`,
        toolName: "bash",
      }),
    ),
    ...[0, 1, 2, 3, 4, 5].map((index) =>
      piece(`new-${index}`, "tool", 500, {
        toolUseId: `n${index}`,
        toolName: "bash",
      }),
    ),
  ];
  expect(chunks.reduce((sum, chunk) => sum + chunk.tokens, 0)).toBe(30000);
  const { decisions, used, overrun } = eliminate(chunks, {
    keepRecent: 6,
    remainingBudget: 10000,
  });
  expect(decisions).toEqual([
    { id: "seq:1", tier: "full", reason: "kept" },
    { id: "seq:2", tier: "full", reason: "kept" },
    { id: "old-0", tier: "none", reason: "budget:oldest-first" },
    { id: "old-1", tier: "none", reason: "budget:oldest-first" },
    { id: "old-2", tier: "none", reason: "budget:oldest-first" },
    { id: "old-3", tier: "none", reason: "budget:oldest-first" },
    { id: "old-4", tier: "full", reason: "kept" },
    { id: "new-0", tier: "full", reason: "recent" },
    { id: "new-1", tier: "full", reason: "recent" },
    { id: "new-2", tier: "full", reason: "recent" },
    { id: "new-3", tier: "full", reason: "recent" },
    { id: "new-4", tier: "full", reason: "recent" },
    { id: "new-5", tier: "full", reason: "recent" },
  ]);
  expect(used).toBe(10000);
  expect(overrun).toBe(0);
});

it("never demotes user or assistant prose for budget and reports the overrun", () => {
  const chunks: Chunk[] = [
    piece("seq:1", "user", 8000),
    piece("seq:2", "assistant", 8000),
    piece("old", "tool", 4000, { toolUseId: "o", toolName: "bash" }),
    piece("new", "tool", 100, { toolUseId: "n", toolName: "bash" }),
  ];
  const { decisions, used, overrun } = eliminate(chunks, {
    keepRecent: 1,
    remainingBudget: 10000,
  });
  expect(decisions).toEqual([
    { id: "seq:1", tier: "full", reason: "kept" },
    { id: "seq:2", tier: "full", reason: "kept" },
    { id: "old", tier: "none", reason: "budget:oldest-first" },
    { id: "new", tier: "full", reason: "recent" },
  ]);
  expect(used).toBe(16100);
  expect(overrun).toBe(6100);
});

it("keeps a superseded reason when the later chunk is then dropped for budget", () => {
  const chunks: Chunk[] = [
    piece("seq:1", "user", 5000),
    piece("seq:2#old", "tool", 100, {
      toolUseId: "old",
      toolName: "read",
      accesses: [{ key: "/repo/a.ts", mode: "read" }],
    }),
    piece("seq:4#new", "tool", 50000, {
      toolUseId: "new",
      toolName: "read",
      accesses: [{ key: "/repo/a.ts", mode: "read" }],
    }),
  ];
  const { decisions, used, overrun } = eliminate(chunks, {
    keepRecent: 0,
    remainingBudget: 1000,
  });
  expect(decisions).toEqual([
    { id: "seq:1", tier: "full", reason: "kept" },
    { id: "seq:2#old", tier: "none", reason: "superseded_by:seq:4#new" },
    { id: "seq:4#new", tier: "none", reason: "budget:oldest-first" },
  ]);
  expect(used).toBe(5000);
  expect(overrun).toBe(4000);
});
