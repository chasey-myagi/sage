import { AgentSession, MemoryLogStore, type ViewContext } from "@sage/core";
import {
  FAUX_UNIT_USAGE,
  fauxAdapter,
  stopScript,
  toolScript,
} from "@sage/core/testing";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { expandTool } from "../../src/compiler/expand.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };

function fixed(name: string, content: string) {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute() {
      return { content, isError: false };
    },
  };
}

function resultsOf(
  messages: {
    content: { type: string; toolUseId?: string; content?: string }[];
  }[],
  toolUseId: string,
) {
  return messages.flatMap((message) =>
    message.content.filter(
      (block) => block.type === "tool_result" && block.toolUseId === toolUseId,
    ),
  );
}

it("returns the logged original for a tool chunk or a prose chunk", async () => {
  const store = new MemoryLogStore();
  const sessionId = "expand";
  store.append(sessionId, {
    kind: "message",
    payload: {
      role: "user",
      content: [
        { type: "thinking", text: "hmm" },
        { type: "text", text: "hello" },
      ],
    },
  });
  store.append(sessionId, {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "t1",
          name: "bash",
          input: { cmd: "ls" },
          rawInput: '{"cmd":"ls"}',
        },
      ],
    },
  });
  store.append(sessionId, {
    kind: "message",
    payload: {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "t1",
          content: "alpha\nbeta",
          isError: true,
        },
      ],
    },
  });
  const tool = expandTool({ store, sessionId });
  expect(tool.def.name).toBe("expand");
  expect(tool.isReadOnly).toBe(true);
  expect(tool.accesses({ id: "seq:1#t1" })).toEqual([]);
  const before = store.all(sessionId).length;
  const ctx = { sessionId, toolUseId: "call" };

  expect(await tool.execute({ id: "seq:1#t1" }, ctx)).toEqual({
    content: "alpha\nbeta",
    isError: false,
  });
  expect(await tool.execute({ id: "seq:0" }, ctx)).toEqual({
    content: "hmmhello",
    isError: false,
  });
  expect(await tool.execute({ id: "seq:2" }, ctx)).toEqual({
    content: "Unknown chunk: seq:2",
    isError: true,
  });
  expect(await tool.execute({ id: "missing" }, ctx)).toEqual({
    content: "Unknown chunk: missing",
    isError: true,
  });
  expect(store.all(sessionId).length).toBe(before);
});

it("keeps the expanded chunk omitted and shows the expand result in full next turn", async () => {
  const store = new MemoryLogStore();
  const sessionId = "next";
  const adapter = fauxAdapter([
    toolScript("a", "read", "{}", FAUX_UNIT_USAGE),
    toolScript("b", "read", "{}", FAUX_UNIT_USAGE),
    toolScript(
      "ex",
      "expand",
      JSON.stringify({ id: "seq:3#a" }),
      FAUX_UNIT_USAGE,
    ),
    stopScript("done", FAUX_UNIT_USAGE),
  ]);
  const session = new AgentSession({
    store,
    sessionId,
    adapter,
    model: MODEL,
    system: "You are sage.",
    tools: [fixed("read", "SECRET"), expandTool({ store, sessionId })],
    viewProvider: contextCompiler({
      budget: 0,
      claims: [],
      keepRecent: 1,
    }),
  });

  const summary = await session.run("look");
  expect(summary).not.toHaveProperty("expands");
  expect(adapter.requests).toHaveLength(4);

  const stub =
    '[#seq:3#a · read · {} · 0.0k tokens · omitted · expand("seq:3#a") to retrieve]';
  const third = adapter.requests[3];
  if (!third) throw new Error("missing fourth request");
  expect(resultsOf(third.messages, "a")).toEqual([
    {
      type: "tool_result",
      toolUseId: "a",
      content: stub,
      isError: false,
    },
  ]);
  expect(resultsOf(third.messages, "ex")).toEqual([
    {
      type: "tool_result",
      toolUseId: "ex",
      content: "SECRET",
      isError: false,
    },
  ]);

  const manifests = store
    .all(sessionId)
    .filter((entry) => entry.kind === "manifest");
  const payload = manifests[3]?.payload as {
    turn?: number;
    chunks?: { id: string; tier: string; reason: string }[];
  };
  expect(payload.turn).toBe(4);
  expect(payload.chunks).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: "seq:3#a",
        tier: "none",
        reason: "budget:oldest-first;expanded_at:3",
      }),
      expect.objectContaining({
        id: "seq:12#ex",
        tier: "full",
        reason: "recent",
      }),
    ]),
  );
});

it("counts omitted chunks and expand calls on the manifest budget", async () => {
  const store = new MemoryLogStore();
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "go" }],
    },
  });
  for (let index = 0; index < 10; index += 1) {
    const id = `t${index}`;
    store.append("s", {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id,
            name: "read",
            input: { path: `/${index}.txt` },
            rawInput: "{}",
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
            toolUseId: id,
            content: "body",
            isError: false,
          },
        ],
      },
    });
  }
  for (let index = 0; index < 2; index += 1) {
    store.append("s", {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: `e${index}`,
            name: "expand",
            input: { id: "nope" },
            rawInput: "{}",
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
            toolUseId: `e${index}`,
            content: "x",
            isError: false,
          },
        ],
      },
    });
  }
  const ctx: ViewContext = {
    sessionId: "s",
    turn: 9,
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
    keepRecent: 2,
  }).view(ctx);
  expect(view.manifest.budget).toEqual(
    expect.objectContaining({ omitted: 10, expands: 2 }),
  );
});

it("reads parent and own chunks through a child: id", async () => {
  const store = new MemoryLogStore();
  store.append("parent", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "PARENT-TEXT" }],
    },
  });
  const leaf = store.leaf("parent");
  if (!leaf) throw new Error("missing parent leaf");
  store.append("child", {
    kind: "subtask",
    payload: {
      parentSessionId: "parent",
      parentLeaf: leaf,
      query: "q",
      budget: 10,
      readOnly: true,
    },
  });
  store.append("child", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "OWN-TEXT" }],
    },
  });
  const tool = expandTool({ store, sessionId: "parent" });
  const ctx = { sessionId: "parent", toolUseId: "ex" };
  expect(await tool.execute({ id: "child:child#seq:0" }, ctx)).toEqual({
    content: "PARENT-TEXT",
    isError: false,
  });
  expect(await tool.execute({ id: "child:child#own:seq:1" }, ctx)).toEqual({
    content: "OWN-TEXT",
    isError: false,
  });
});
