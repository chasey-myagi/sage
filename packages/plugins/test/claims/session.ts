/**
 * Builds a real session path for claim-provider tests.
 *
 * The faux adapter never opens a socket. Claim providers read this path;
 * they are not given a hand-built message array.
 */

import {
  AgentSession,
  type HarnessTool,
  MemoryLogStore,
  type ViewContext,
  type ViewProvider,
} from "@sage/core";
import { fauxAdapter } from "@sage/core/testing";
import type { StreamChunk } from "@sage/provider/contract";

export const MODEL = { provider: "anthropic", model: "claude-test" };

const USAGE = {
  inputOther: 3,
  inputCacheRead: 1,
  inputCacheCreation: 0,
  output: 2,
};

export function stopScript(text: string): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text },
    { type: "block_end", index: 0 },
    { type: "usage", usage: { ...USAGE } },
    { type: "finish", kind: "stop" },
  ];
}

export function toolScript(
  id: string,
  name: string,
  inputJson = "{}",
): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "tool_use", id, name },
    { type: "tool_input_delta", index: 0, partialJson: inputJson },
    { type: "block_end", index: 0 },
    { type: "usage", usage: { ...USAGE } },
    { type: "finish", kind: "tool_use" },
  ];
}

export async function runSession(opts: {
  scripts: StreamChunk[][];
  tools?: HarnessTool[];
  viewProvider?: ViewProvider;
  sessionId?: string;
}) {
  const store = new MemoryLogStore();
  const sessionId = opts.sessionId ?? "claims";
  const adapter = fauxAdapter(opts.scripts);
  const session = new AgentSession({
    store,
    sessionId,
    adapter,
    model: MODEL,
    system: "You are sage.",
    tools: opts.tools ?? [],
    viewProvider: opts.viewProvider,
  });
  await session.run("task");
  return { store, sessionId, adapter };
}

export function contextOf(
  store: MemoryLogStore,
  sessionId: string,
  turn = 1,
): ViewContext {
  return {
    sessionId,
    turn,
    path: store.path(sessionId),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 8000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
  };
}
