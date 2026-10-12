import { expect, it } from "vitest";
import { MemoryLogStore, passthroughView } from "../../src/index.ts";
import { FAUX_MODEL as MODEL } from "../../src/testing/index.ts";

const prepared = {
  model: MODEL,
  contextWindow: 100,
  pricing: null,
  adapterDefaults: {},
};

function toolUse(id: string) {
  return {
    type: "tool_use" as const,
    id,
    name: "bash",
    input: {},
    rawInput: "{}",
  };
}

it("names each passthrough chunk after its source entry and gives synthesized messages ids that name no entry", async () => {
  const store = new MemoryLogStore();
  const sessionId = "chunk-ids";
  const user = store.append(sessionId, {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "go" }] },
  });
  const first = store.append(sessionId, {
    kind: "message",
    payload: { role: "assistant", content: [toolUse("call_a")] },
  });
  const second = store.append(sessionId, {
    kind: "message",
    payload: { role: "assistant", content: [toolUse("call_b")] },
  });

  const view = await passthroughView.view({
    sessionId,
    turn: 1,
    path: store.path(sessionId),
    model: MODEL,
    prepared,
    head: { system: "sys", tools: [] },
    tools: [],
    depth: 0,
  });

  expect(view.messages.map((message) => message.role)).toEqual([
    "user",
    "assistant",
    "user",
    "assistant",
    "user",
  ]);
  expect(view.manifest.chunks.map((chunk) => chunk.id)).toEqual([
    user.id,
    first.id,
    `dangling-before:${second.id}`,
    second.id,
    "dangling:call_b",
  ]);
  const entryIds = new Set<string>(
    store.path(sessionId).map((entry) => entry.id),
  );
  expect(entryIds.has(`dangling-before:${second.id}`)).toBe(false);
  expect(entryIds.has("dangling:call_b")).toBe(false);
});
