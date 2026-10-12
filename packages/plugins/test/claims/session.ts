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
import {
  FAUX_CONTEXT_WINDOW,
  FAUX_MODEL,
  FAUX_SYSTEM,
  fauxAdapter,
} from "@sage/core/testing";
import type { StreamChunk } from "@sage/provider/contract";

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
    adapters: { anthropic: adapter },
    model: FAUX_MODEL,
    system: FAUX_SYSTEM,
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
    model: FAUX_MODEL,
    prepared: {
      model: FAUX_MODEL,
      contextWindow: FAUX_CONTEXT_WINDOW,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: FAUX_SYSTEM, tools: [] },
    tools: [],
  };
}
