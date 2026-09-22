/**
 * Fixture provider. Plays a script of chunks, one inner array per turn.
 *
 * Contract: `prepare` returns the partial call filled in with the model the
 * session asked for. `stream` records a structured clone of each request,
 * then yields that turn's chunks. It does not open a socket. Running out of
 * turns throws. An already-aborted signal finishes as `aborted`.
 *
 * Intentionally not in the Log: the script and the captured requests. The
 * session writes the request envelope and the collected assistant message.
 */

import type {
  PreparedCall,
  ProviderAdapter,
  Request,
  StreamChunk,
} from "@sage/provider/contract";

export function fauxAdapter(
  script: StreamChunk[][],
  opts?: { api?: ProviderAdapter["api"]; prepared?: Partial<PreparedCall> },
): ProviderAdapter & { requests: Request[] } {
  const requests: Request[] = [];
  let cursor = 0;
  const api = opts?.api ?? "anthropic-messages";
  const seeded = opts?.prepared;

  const adapter: ProviderAdapter & { requests: Request[] } = {
    api,
    requests,
    prepare(model) {
      return {
        model,
        contextWindow: seeded?.contextWindow ?? 8000,
        pricing: seeded?.pricing === undefined ? null : seeded.pricing,
        adapterDefaults: seeded?.adapterDefaults ?? {},
      };
    },
    stream(request, _prepared, signal) {
      requests.push(structuredClone(request));
      const turn = script[cursor];
      const index = cursor;
      cursor += 1;
      if (!turn) {
        throw new Error(`faux adapter ran out of scripts at call ${index}`);
      }
      return (async function* () {
        if (signal?.aborted) {
          yield { type: "finish" as const, kind: "aborted" as const };
          return;
        }
        for (const chunk of turn) yield chunk;
      })();
    },
  };
  return adapter;
}
