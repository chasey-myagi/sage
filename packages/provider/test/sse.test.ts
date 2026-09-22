import { describe, expect, it } from "vitest";
import { parseSse } from "../src/sse.ts";

function bodyFrom(text: string): ReadableStream<Uint8Array> {
  const body = new Response(text).body;
  if (!body) throw new Error("missing body");
  return body;
}

async function events(
  text: string,
): Promise<{ event: string; data: string }[]> {
  const out: { event: string; data: string }[] = [];
  for await (const event of parseSse(bodyFrom(text))) out.push(event);
  return out;
}

describe("sse", () => {
  it("joins multiline data, skips comments, and drops a partial tail", async () => {
    const text = [
      ": keep-alive",
      "",
      "event: message",
      "data: hello",
      "data: world",
      "",
      "data:next",
      "",
      "data: trunca",
    ].join("\n");
    expect(await events(text)).toEqual([
      { event: "message", data: "hello\nworld" },
      { event: "", data: "next" },
    ]);
  });

  it("splits events on CRLF blank lines", async () => {
    const text = "data: one\r\n\r\ndata: two\r\n\r\n";
    expect(await events(text)).toEqual([
      { event: "", data: "one" },
      { event: "", data: "two" },
    ]);
  });
});
