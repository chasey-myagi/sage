/**
 * A scripted `fetch` double that answers every call with a 200 SSE body,
 * for driving a real provider adapter without the network.
 *
 * Guarantees: never throws and never reads the request; each call gets
 * the next body in order.
 *
 * Intentionally not in the Log: the double itself. The adapter it feeds
 * produces whatever the session logs.
 */

/**
 * A `fetch` that answers call n with `bodies[n]` as a 200
 * `text/event-stream` response, repeating the last body once the script
 * runs out, so a test can script "fail forever" with one body. `calls`
 * returns how many times it was called. Throws if `bodies` is empty.
 */
export function scriptedSseFetch(bodies: string[]): {
  fetchImpl: typeof fetch;
  calls: () => number;
} {
  const last = bodies.at(-1);
  if (last === undefined) throw new Error("scriptedSseFetch needs a body");
  let count = 0;
  return {
    fetchImpl: async () => {
      const body = bodies[count] ?? last;
      count += 1;
      return new Response(body, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
    },
    calls: () => count,
  };
}
