/**
 * A scripted `fetch` double that answers each call with the next scripted
 * response, for driving a real provider adapter without the network.
 *
 * Guarantees: each call gets the next step in order; it throws only when
 * that step says so. The request body is recorded as sent, never
 * interpreted.
 *
 * Intentionally not in the Log: the double itself. The adapter it feeds
 * produces whatever the session logs.
 */

import { assertNever } from "@sage/provider/contract";

/**
 * One scripted response. A string is a 200 `text/event-stream` body that
 * closes normally. `no-body` is a 200 without a body. `throw` makes fetch
 * itself throw a `TypeError` with `message`, as a failed connection does.
 * `cut` answers `status` (200 when omitted), sends `body`, then errors the
 * body read with a `TypeError` carrying `message`, as a dropped connection
 * does.
 */
export type SseFetchStep =
  | string
  | { kind: "no-body" }
  | { kind: "throw"; message: string }
  | { kind: "cut"; body: string; message: string; status?: number };

/**
 * A `fetch` that answers call n with `steps[n]`, repeating the last step
 * once the script runs out, so a test can script "fail forever" with one
 * step. `calls` returns how many times it was called; `requestBodies`
 * returns each call's request body text in call order ("" when the body
 * was not a string). Throws if `steps` is empty.
 */
export function scriptedSseFetch(steps: SseFetchStep[]): {
  fetchImpl: typeof fetch;
  calls: () => number;
  requestBodies: () => string[];
} {
  const last = steps.at(-1);
  if (last === undefined) throw new Error("scriptedSseFetch needs a step");
  const sent: string[] = [];
  return {
    fetchImpl: async (_input, init) => {
      const step = steps[sent.length] ?? last;
      sent.push(typeof init?.body === "string" ? init.body : "");
      return respond(step);
    },
    calls: () => sent.length,
    requestBodies: () => [...sent],
  };
}

const SSE_HEADERS = { "content-type": "text/event-stream" };

function respond(step: SseFetchStep): Response {
  if (typeof step === "string") {
    return new Response(step, { status: 200, headers: SSE_HEADERS });
  }
  switch (step.kind) {
    case "no-body":
      return new Response(null, { status: 200, headers: SSE_HEADERS });
    case "throw":
      throw new TypeError(step.message);
    case "cut":
      return new Response(cutBody(step.body, step.message), {
        status: step.status ?? 200,
        headers: SSE_HEADERS,
      });
    default:
      return assertNever(step);
  }
}

/** A body that yields `body`, then fails the next read with `message`. */
function cutBody(body: string, message: string): ReadableStream<Uint8Array> {
  const bytes = new TextEncoder().encode(body);
  let sent = false;
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      if (!sent) {
        sent = true;
        controller.enqueue(bytes);
        return;
      }
      controller.error(new TypeError(message));
    },
  });
}
