import { OpenAIAdapter } from "@sage/provider";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  createFaux,
  FAUX_SYSTEM as SYSTEM,
  stopScript,
} from "../../src/testing/index.ts";

const MODEL = { provider: "openai", model: "gpt-5" };

const PARTIAL = [
  'data: {"id":"c1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"half-"},"finish_reason":null}]}',
  "",
  "",
].join("\n");

const OK = [
  'data: {"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}]}',
  "",
  'data: {"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"c2","object":"chat.completion.chunk","model":"gpt-5","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
  "",
].join("\n");

type Step =
  | { action: "throw"; message: string }
  | { action: "cut"; body: string; message: string }
  | { action: "sse"; body: string };

function scriptedFetch(steps: Step[]): { fetchImpl: typeof fetch } {
  let call = 0;
  const fetchImpl: typeof fetch = async () => {
    const step = steps[call];
    call += 1;
    if (!step) throw new Error(`unexpected fetch call ${call}`);
    if (step.action === "throw") throw new TypeError(step.message);
    if (step.action === "sse") {
      return new Response(step.body, {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      });
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
    return new Response(body, {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  };
  return { fetchImpl };
}

async function runWith(sessionId: string, steps: Step[]) {
  const store = new MemoryLogStore();
  const { fetchImpl } = scriptedFetch(steps);
  const adapter = new OpenAIAdapter({
    apiKey: "k",
    fetchImpl,
    sleep: async () => undefined,
  });
  const session = new AgentSession({
    store,
    sessionId,
    adapter,
    model: MODEL,
    system: SYSTEM,
    tools: [],
    sleep: async () => undefined,
  });
  const summary = await session.run("go");
  return { summary, path: store.path(sessionId) };
}

describe("transport flag through the real adapter pipeline", () => {
  it("retries the turn when the body read fails mid-stream with an unlisted message", async () => {
    const { path } = await runWith("mid-stream", [
      { action: "cut", body: PARTIAL, message: "other side closed" },
      { action: "sse", body: OK },
    ]);

    expect(
      path
        .filter((entry) => entry.kind === "turn_retry")
        .map((entry) => entry.payload),
    ).toEqual([{ turn: 1, attempt: 1, reason: "other side closed" }]);
    expect(path.at(-1)?.payload).toMatchObject({
      outcome: "done",
      health: { turnRetries: 1 },
    });
    expect(JSON.stringify(path)).not.toContain("half-");
  });

  it("retries the turn when fetch keeps throwing past the provider budget", async () => {
    const { path } = await runWith("pre-body", [
      { action: "throw", message: "getaddrinfo EAI_AGAIN api.example" },
      { action: "throw", message: "getaddrinfo EAI_AGAIN api.example" },
      { action: "throw", message: "getaddrinfo EAI_AGAIN api.example" },
      { action: "sse", body: OK },
    ]);

    expect(
      path
        .filter((entry) => entry.kind === "turn_retry")
        .map((entry) => entry.payload),
    ).toEqual([
      { turn: 1, attempt: 1, reason: "getaddrinfo EAI_AGAIN api.example" },
    ]);
    expect(path.at(-1)?.payload).toMatchObject({
      outcome: "done",
      health: { turnRetries: 1 },
    });
  });

  it("does not read transport from the message text", async () => {
    const store = new MemoryLogStore();
    const { adapter } = createFaux([
      [
        { type: "block_start", index: 0, blockType: "text" },
        { type: "text_delta", index: 0, text: "cut" },
        { type: "finish", kind: "error", error: { message: "terminated" } },
      ],
      stopScript("unused"),
    ]);
    const session = new AgentSession({
      store,
      sessionId: "text-only",
      adapter,
      model: { provider: "anthropic", model: "claude-test" },
      system: SYSTEM,
      tools: [],
      sleep: async () => undefined,
    });
    await session.run("go");
    const path = store.path("text-only");

    expect(path.filter((entry) => entry.kind === "turn_retry")).toEqual([]);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "error" });
  });
});
