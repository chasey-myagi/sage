export const MODEL = { provider: "anthropic", model: "claude-test" };

export const SYSTEM = "You are sage.";

export const USAGE = {
  inputOther: 3,
  inputCacheRead: 1,
  inputCacheCreation: 0,
  output: 2,
};

export type FauxChunk =
  | {
      type: "block_start";
      index: number;
      blockType: string;
      id?: string;
      name?: string;
    }
  | { type: "text_delta"; index: number; text: string }
  | { type: "thinking_delta"; index: number; text: string }
  | { type: "tool_input_delta"; index: number; partialJson: string }
  | { type: "block_end"; index: number }
  | { type: "usage"; usage: typeof USAGE }
  | {
      type: "finish";
      kind: string;
      rawFinishReason?: string;
      error?: { message: string; status?: number; raw?: unknown };
    };

export type FauxRequest = {
  model: { provider: string; model: string };
  head: {
    system: string;
    tools: Array<{
      name: string;
      description: string;
      inputSchema: Record<string, unknown>;
    }>;
  };
  messages: Array<{ role: string; content: unknown[] }>;
  cacheBreakpoints: unknown[];
  sampling: { maxTokens?: number; temperature?: number };
};

export type FauxPrepared = {
  model: typeof MODEL;
  contextWindow: number;
  pricing: null;
  adapterDefaults: Record<string, unknown>;
};

type Script =
  | FauxChunk[]
  | ((ctx: {
      request: FauxRequest;
      signal?: AbortSignal;
      index: number;
    }) => AsyncIterable<FauxChunk>);

export function stopScript(text: string): FauxChunk[] {
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
): FauxChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "tool_use", id, name },
    { type: "tool_input_delta", index: 0, partialJson: inputJson },
    { type: "block_end", index: 0 },
    { type: "usage", usage: { ...USAGE } },
    { type: "finish", kind: "tool_use" },
  ];
}

export function createFaux(
  scripts: Script[],
  defaults?: Record<string, unknown>,
) {
  const requests: FauxRequest[] = [];
  let cursor = 0;
  const prepared: FauxPrepared = {
    model: MODEL,
    contextWindow: 8000,
    pricing: null,
    adapterDefaults: defaults ?? { temperature: 0, maxTokens: 64 },
  };
  const adapter = {
    api: "anthropic-messages" as const,
    prepare() {
      return prepared;
    },
    stream(
      request: FauxRequest,
      _prepared: FauxPrepared,
      signal?: AbortSignal,
    ) {
      requests.push(structuredClone(request));
      const script = scripts[cursor];
      const index = cursor;
      cursor += 1;
      if (!script) {
        throw new Error(`faux adapter ran out of scripts at call ${index}`);
      }
      if (typeof script === "function")
        return script({ request, signal, index });
      return (async function* () {
        if (signal?.aborted) {
          yield { type: "finish" as const, kind: "aborted" };
          return;
        }
        for (const chunk of script) yield chunk;
      })();
    },
  };
  return { adapter, requests, prepared };
}
