/**
 * A scripted `fetch` double that answers every call with a 200 SSE body,
 * for driving a real provider adapter without the network.
 *
 * Guarantees: never throws; each call gets the next body in order. The
 * request body is recorded as sent, never interpreted.
 *
 * Intentionally not in the Log: the double itself. The adapter it feeds
 * produces whatever the session logs.
 */

/**
 * A `fetch` that answers call n with `bodies[n]` as a 200
 * `text/event-stream` response, repeating the last body once the script
 * runs out, so a test can script "fail forever" with one body. `calls`
 * returns how many times it was called; `requestBodies` returns each
 * call's request body text in call order ("" when the body was not a
 * string). Throws if `bodies` is empty.
 */
export function scriptedSseFetch(bodies: string[]): {
  fetchImpl: typeof fetch;
  calls: () => number;
  requestBodies: () => string[];
} {
  const last = bodies.at(-1);
  if (last === undefined) throw new Error("scriptedSseFetch needs a body");
  const sent: string[] = [];
  return {
    fetchImpl: async (_input, init) => {
      const body = bodies[sent.length] ?? last;
      sent.push(typeof init?.body === "string" ? init.body : "");
      return new Response(body, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    },
    calls: () => sent.length,
    requestBodies: () => [...sent],
  };
}
