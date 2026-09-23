/**
 * Fixture provider: plays a script, one entry per model call, and records
 * each request. The one test double for the provider seam.
 *
 * Guarantees: `stream` records a structured clone of each request, then
 * plays that call's script. It never opens a socket. A script without a
 * response chunk collects to empty responseId and responseModel. Running
 * out of scripts throws. An already-aborted signal finishes as `aborted`
 * (array scripts only; a function script decides for itself).
 *
 * Intentionally not in the Log: the scripts and the captured requests. The
 * session writes the request envelope and the collected assistant message.
 */

import type {
  ModelRef,
  PreparedCall,
  ProviderAdapter,
  Request,
  StreamChunk,
  TokenCounts,
} from "@sage/provider/contract";

/** Context window, in tokens, a faux prepared call reports by default. */
export const FAUX_CONTEXT_WINDOW = 8000;

/** Model the `createFaux` fixtures run as. */
export const FAUX_MODEL: ModelRef = {
  provider: "anthropic",
  model: "claude-test",
};

/** System prompt the session fixtures use. */
export const FAUX_SYSTEM = "You are sage.";

/** Usage `stopScript` and `toolScript` report unless given another. */
export const FAUX_USAGE: TokenCounts = {
  inputOther: 3,
  inputCacheRead: 1,
  inputCacheCreation: 0,
  output: 2,
};

/**
 * One uncached input token and one output token: for tests that want usage
 * present but small enough not to move token-driven decisions.
 */
export const FAUX_UNIT_USAGE: TokenCounts = {
  inputOther: 1,
  inputCacheRead: 0,
  inputCacheCreation: 0,
  output: 1,
};

/**
 * One call's script: the chunks to yield, or a function that produces the
 * stream itself (for delays, aborts, or request-dependent replies). `index`
 * is the 0-based call number.
 */
export type FauxScript =
  | StreamChunk[]
  | ((ctx: {
      request: Request;
      signal?: AbortSignal;
      index: number;
    }) => AsyncIterable<StreamChunk>);

/**
 * A faux adapter whose `prepare` fills in the requested model with
 * `opts.prepared` (context window `FAUX_CONTEXT_WINDOW`, null pricing, and
 * empty adapter defaults unless given). `requests` fills as calls are made.
 */
export function fauxAdapter(
  scripts: FauxScript[],
  opts?: { api?: ProviderAdapter["api"]; prepared?: Partial<PreparedCall> },
): ProviderAdapter & { requests: Request[] } {
  const requests: Request[] = [];
  let cursor = 0;
  const seeded = opts?.prepared;

  return {
    api: opts?.api ?? "anthropic-messages",
    requests,
    prepare(model) {
      return {
        model,
        contextWindow: seeded?.contextWindow ?? FAUX_CONTEXT_WINDOW,
        pricing: seeded?.pricing === undefined ? null : seeded.pricing,
        adapterDefaults: seeded?.adapterDefaults ?? {},
      };
    },
    stream(request, _prepared, signal) {
      requests.push(structuredClone(request));
      const index = cursor;
      const script = scripts[index];
      cursor += 1;
      if (!script) {
        throw new Error(`faux adapter ran out of scripts at call ${index}`);
      }
      if (typeof script === "function") {
        return script({ request, signal, index });
      }
      return (async function* () {
        if (signal?.aborted) {
          yield { type: "finish" as const, kind: "aborted" as const };
          return;
        }
        for (const chunk of script) yield chunk;
      })();
    },
  };
}

/**
 * Session-test fixture: a faux adapter whose `prepare` always returns the
 * same `prepared` object, labelled `FAUX_MODEL`, whatever model is asked
 * for. `defaults` becomes its adapter defaults (`{temperature: 0,
 * maxTokens: 64}` when omitted).
 */
export function createFaux(
  scripts: FauxScript[],
  defaults?: Record<string, unknown>,
): {
  adapter: ProviderAdapter & { requests: Request[] };
  requests: Request[];
  prepared: PreparedCall;
} {
  const prepared: PreparedCall = {
    model: FAUX_MODEL,
    contextWindow: FAUX_CONTEXT_WINDOW,
    pricing: null,
    adapterDefaults: defaults ?? { temperature: 0, maxTokens: 64 },
  };
  const adapter = { ...fauxAdapter(scripts), prepare: () => prepared };
  return { adapter, requests: adapter.requests, prepared };
}

/**
 * A call that streams one text block and stops. `usage` (default
 * `FAUX_USAGE`) is copied into the usage chunk.
 */
export function stopScript(
  text: string,
  usage: TokenCounts = FAUX_USAGE,
): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text },
    { type: "block_end", index: 0 },
    { type: "usage", usage: { ...usage } },
    { type: "finish", kind: "stop" },
  ];
}

/**
 * A call that streams one tool_use block and finishes with `tool_use`.
 * `usage` (default `FAUX_USAGE`) is copied into the usage chunk.
 */
export function toolScript(
  id: string,
  name: string,
  inputJson = "{}",
  usage: TokenCounts = FAUX_USAGE,
): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "tool_use", id, name },
    { type: "tool_input_delta", index: 0, partialJson: inputJson },
    { type: "block_end", index: 0 },
    { type: "usage", usage: { ...usage } },
    { type: "finish", kind: "tool_use" },
  ];
}
