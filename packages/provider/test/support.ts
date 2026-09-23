import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { StreamChunk, Usage } from "../src/contract/index.ts";

const fixturesDir = fileURLToPath(new URL("./fixtures/", import.meta.url));

export function readFixture(name: string): string {
  return readFileSync(`${fixturesDir}${name}`, "utf8");
}

export function stripRaw(chunks: StreamChunk[]): StreamChunk[] {
  return chunks.map((chunk) => {
    if (chunk.type !== "usage") return chunk;
    return { type: "usage", usage: quadruple(chunk.usage) };
  });
}

export function quadruple(usage: Usage): Usage {
  return {
    inputOther: usage.inputOther,
    inputCacheRead: usage.inputCacheRead,
    inputCacheCreation: usage.inputCacheCreation,
    output: usage.output,
  };
}

export async function drain(
  chunks: AsyncIterable<StreamChunk>,
): Promise<StreamChunk[]> {
  const out: StreamChunk[] = [];
  for await (const chunk of chunks) out.push(chunk);
  return out;
}

export type RecordedCall = {
  url: string;
  method: string;
  headers: Headers;
  body: unknown;
};

export function fixtureFetch(
  sse: string,
  status = 200,
): { fetchImpl: typeof fetch; calls: RecordedCall[] } {
  const calls: RecordedCall[] = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    calls.push({
      url: String(input),
      method: init?.method ?? "GET",
      headers: new Headers(init?.headers),
      body: JSON.parse(String(init?.body ?? "null")) as unknown,
    });
    return new Response(sse, {
      status,
      headers: { "content-type": "text/event-stream" },
    });
  };
  return { fetchImpl, calls };
}

/**
 * One scripted fetch outcome: fetch throws; a response whose body is read
 * in full; or a response that sends `body` and then fails the read with
 * `message`.
 */
export type FetchStep =
  | { action: "throw"; message: string }
  | { action: "respond"; status: number; body: string | null }
  | { action: "cut"; status: number; body: string; message: string };

/** Plays `steps` in order, one per call; the last step repeats. */
export function scriptedFetch(steps: FetchStep[]): {
  fetchImpl: typeof fetch;
  calls: () => number;
} {
  let call = 0;
  const fetchImpl: typeof fetch = async () => {
    const step = steps[Math.min(call, steps.length - 1)];
    call += 1;
    if (!step) throw new Error("scriptedFetch needs at least one step");
    if (step.action === "throw") throw new TypeError(step.message);
    const headers = { "content-type": "text/event-stream" };
    if (step.action === "respond") {
      return new Response(step.body, { status: step.status, headers });
    }
    const bytes = new TextEncoder().encode(step.body);
    let sent = false;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        if (!sent) {
          sent = true;
          controller.enqueue(bytes);
          return;
        }
        controller.error(new TypeError(step.message));
      },
    });
    return new Response(body, { status: step.status, headers });
  };
  return { fetchImpl, calls: () => call };
}
