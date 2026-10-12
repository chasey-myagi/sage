/**
 * A recording `fetch` double for JSON-over-HTTP clients such as the jev
 * scorer: it parses each request body and answers from a callback.
 *
 * Guarantees: every call is recorded in call order before the callback
 * runs, and the callback sees the parsed body. A body that is not a string
 * or not JSON is recorded as `undefined`. The double never throws on its
 * own; a callback that throws makes that fetch reject.
 *
 * Intentionally not in the Log: the double and its records. The client it
 * feeds decides what reaches the Log.
 */

/** One recorded call. `body` is the parsed JSON body, or undefined. */
export type JsonFetchRequest = {
  url: string;
  method?: string;
  headers: Record<string, string>;
  body: unknown;
};

/**
 * How the double answers one call: `json` is serialized as a JSON body
 * with status `status` (200 when omitted); `text` is sent verbatim.
 * `headers` are added to the response, such as `retry-after`.
 */
export type JsonFetchReply =
  | { status?: number; json: unknown; headers?: Record<string, string> }
  | { status: number; text: string; headers?: Record<string, string> };

/**
 * A `fetch` that records each call and answers with `respond(request,
 * index)`, where `index` counts calls from 0. `requests` returns a copy of
 * the records in call order.
 */
export function jsonFetch(
  respond: (
    request: JsonFetchRequest,
    index: number,
  ) => JsonFetchReply | Promise<JsonFetchReply>,
): {
  fetchImpl: (
    url: string,
    init?: {
      method?: string;
      headers?: Record<string, string>;
      body?: string;
      signal?: AbortSignal;
    },
  ) => Promise<Response>;
  requests: () => JsonFetchRequest[];
} {
  const seen: JsonFetchRequest[] = [];
  return {
    fetchImpl: async (url, init) => {
      const request: JsonFetchRequest = {
        url,
        ...(init?.method !== undefined ? { method: init.method } : {}),
        headers: { ...init?.headers },
        body: parseBody(init?.body),
      };
      const index = seen.length;
      seen.push(request);
      return toResponse(await respond(request, index));
    },
    requests: () => [...seen],
  };
}

function parseBody(body: string | undefined): unknown {
  if (body === undefined) return undefined;
  try {
    return JSON.parse(body);
  } catch {
    // A non-JSON body is recorded as undefined; the caller asserts on that.
    return undefined;
  }
}

function toResponse(reply: JsonFetchReply): Response {
  if ("text" in reply) {
    return new Response(reply.text, {
      status: reply.status,
      headers: { ...reply.headers },
    });
  }
  return new Response(JSON.stringify(reply.json), {
    status: reply.status ?? 200,
    headers: { "Content-Type": "application/json", ...reply.headers },
  });
}
