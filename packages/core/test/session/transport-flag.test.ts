import { OpenAIAdapter } from "@sage/provider";
import { describe, expect, it } from "vitest";
import { AgentSession, MemoryLogStore } from "../../src/index.ts";
import {
  createFaux,
  type SseFetchStep,
  FAUX_SYSTEM as SYSTEM,
  scriptedSseFetch,
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

async function runWith(sessionId: string, steps: SseFetchStep[]) {
  const store = new MemoryLogStore();
  const { fetchImpl, calls } = scriptedSseFetch(steps);
  const adapter = new OpenAIAdapter({
    apiKey: "k",
    fetchImpl,
    sleep: async () => undefined,
  });
  const session = new AgentSession({
    store,
    sessionId,
    adapters: { openai: adapter },
    model: MODEL,
    system: SYSTEM,
    tools: [],
    sleep: async () => undefined,
  });
  const summary = await session.run("go");
  return { summary, path: store.path(sessionId), calls: calls() };
}

describe("transport flag through the real adapter pipeline", () => {
  it("retries the turn when the body read fails mid-stream with an unlisted message", async () => {
    const { path, calls } = await runWith("mid-stream", [
      { kind: "cut", body: PARTIAL, message: "other side closed" },
      OK,
    ]);

    expect(calls).toBe(2);
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
    const { path, calls } = await runWith("pre-body", [
      { kind: "throw", message: "getaddrinfo EAI_AGAIN api.example" },
      { kind: "throw", message: "getaddrinfo EAI_AGAIN api.example" },
      { kind: "throw", message: "getaddrinfo EAI_AGAIN api.example" },
      OK,
    ]);

    expect(calls).toBe(4);
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

  it("retries when the connection drops while reading an error response body", async () => {
    const { path, calls } = await runWith("error-body", [
      { kind: "cut", body: "upstream", message: "terminated", status: 502 },
      OK,
    ]);

    expect(calls).toBe(2);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "done" });
  });

  it("does not read transport from the message text", async () => {
    const store = new MemoryLogStore();
    const { adapter } = createFaux([
      [
        { type: "block_start", index: 0, blockType: "text" },
        { type: "text_delta", index: 0, text: "cut" },
        {
          type: "finish",
          kind: "error",
          error: { message: "terminated", cause: "other" },
        },
      ],
      stopScript("unused"),
    ]);
    const session = new AgentSession({
      store,
      sessionId: "text-only",
      adapters: { anthropic: adapter },
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
