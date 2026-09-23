import { expect, it } from "vitest";
import { AnthropicAdapter } from "../../src/anthropic/index.ts";
import type { Request } from "../../src/contract/index.ts";
import { OpenAIAdapter } from "../../src/openai/index.ts";
import { drain, fixtureFetch } from "../support.ts";

const OPENAI_SSE = [
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"c","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
].join("\n");

const ANTHROPIC_SSE = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_s","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
  "",
  "event: message_delta",
  'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
  "",
  "event: message_stop",
  'data: {"type":"message_stop"}',
  "",
].join("\n");

function request(toolChoice?: "auto" | "none"): Request {
  return {
    model: { provider: "openai", model: "m" },
    head: {
      system: "You are sage.",
      tools: [
        {
          name: "edit",
          description: "edit a file",
          inputSchema: { type: "object" },
        },
      ],
    },
    messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
    cacheBreakpoints: ["head"],
    sampling: { temperature: 0 },
    ...(toolChoice ? { toolChoice } : {}),
  };
}

function asRecord(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("expected object");
  }
  return value as Record<string, unknown>;
}

async function openaiBody(
  toolChoice?: "auto" | "none",
): Promise<Record<string, unknown>> {
  const { fetchImpl, calls } = fixtureFetch(OPENAI_SSE);
  const adapter = new OpenAIAdapter({ apiKey: "k", fetchImpl });
  const req = request(toolChoice);
  await drain(adapter.stream(req, adapter.prepare(req.model)));
  return asRecord(calls[0]?.body);
}

async function anthropicBody(
  toolChoice?: "auto" | "none",
): Promise<Record<string, unknown>> {
  const { fetchImpl, calls } = fixtureFetch(ANTHROPIC_SSE);
  const adapter = new AnthropicAdapter({ apiKey: "k", fetchImpl });
  const req = request(toolChoice);
  await drain(adapter.stream(req, adapter.prepare(req.model)));
  return asRecord(calls[0]?.body);
}

it("omits tool_choice unless the request sets toolChoice", async () => {
  const openai = await openaiBody();
  const anthropic = await anthropicBody();
  expect(openai).not.toHaveProperty("tool_choice");
  expect(anthropic).not.toHaveProperty("tool_choice");
});

it("maps toolChoice none and auto onto the wire without touching messages or tools", async () => {
  const openaiPlain = await openaiBody();
  const openaiNone = await openaiBody("none");
  const openaiAuto = await openaiBody("auto");
  expect(openaiNone.tool_choice).toBe("none");
  expect(openaiAuto.tool_choice).toBe("auto");
  expect(openaiNone.messages).toEqual(openaiPlain.messages);
  expect(openaiNone.tools).toEqual(openaiPlain.tools);
  expect(openaiAuto.messages).toEqual(openaiPlain.messages);
  expect(openaiAuto.tools).toEqual(openaiPlain.tools);

  const anthropicPlain = await anthropicBody();
  const anthropicNone = await anthropicBody("none");
  const anthropicAuto = await anthropicBody("auto");
  expect(anthropicNone.tool_choice).toEqual({ type: "none" });
  expect(anthropicAuto.tool_choice).toEqual({ type: "auto" });
  expect(anthropicNone.messages).toEqual(anthropicPlain.messages);
  expect(anthropicNone.tools).toEqual(anthropicPlain.tools);
  expect(anthropicNone.system).toEqual(anthropicPlain.system);
  expect(anthropicAuto.messages).toEqual(anthropicPlain.messages);
  expect(anthropicAuto.tools).toEqual(anthropicPlain.tools);
});
