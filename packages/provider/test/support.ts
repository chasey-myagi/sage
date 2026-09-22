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
