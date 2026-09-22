import type { Request } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { renderLlmInput } from "../../src/testing/index.ts";

function request(overrides: Partial<Request> = {}): Request {
  return {
    model: { provider: "anthropic", model: "claude-test" },
    head: { system: "You are sage.", tools: [] },
    messages: [{ role: "user", content: [{ type: "text", text: "hello" }] }],
    cacheBreakpoints: ["head"],
    sampling: { maxTokens: 64 },
    ...overrides,
  };
}

it("renders system, tools, and one line per block", () => {
  const rendered = renderLlmInput(
    request({
      head: {
        system: "You are sage.",
        tools: [
          {
            name: "echo",
            description: "Echo",
            inputSchema: {
              type: "object",
              properties: {
                b: { type: "string" },
                a: { type: "number" },
              },
            },
          },
        ],
      },
      messages: [
        {
          role: "assistant",
          content: [
            { type: "thinking", text: "abcdef" },
            { type: "text", text: "hi" },
            {
              type: "tool_use",
              id: "c1",
              name: "echo",
              input: { a: 1 },
              rawInput: '{"a":1}',
            },
          ],
        },
        {
          role: "user",
          content: [
            {
              type: "tool_result",
              toolUseId: "c1",
              content: "ok",
              isError: false,
            },
          ],
        },
      ],
    }),
  );

  expect(rendered).toBe(`system: You are sage.
tools: [
  {
    "description": "Echo",
    "inputSchema": {
      "properties": {
        "a": {
          "type": "number"
        },
        "b": {
          "type": "string"
        }
      },
      "type": "object"
    },
    "name": "echo"
  }
]
messages:
[assistant] thinking 6 chars
hi
tool_use echo {"a":1}
[user] tool_result c1 ok`);
});

it("prints <last> plus the new messages when the head matches and history is a prefix", () => {
  const first = request();
  const second = request({
    messages: [
      ...first.messages,
      { role: "assistant", content: [{ type: "text", text: "hi" }] },
    ],
  });
  expect(renderLlmInput(second, first)).toBe(`system: <same>
tools: <same>
messages:
<last>
+ [assistant] hi`);
});

it("marks every line of an added multi-block message", () => {
  const first = request();
  const second = request({
    messages: [
      ...first.messages,
      {
        role: "assistant",
        content: [
          { type: "text", text: "hi" },
          {
            type: "tool_use",
            id: "c1",
            name: "echo",
            input: {},
            rawInput: "{}",
          },
        ],
      },
    ],
  });
  expect(renderLlmInput(second, first)).toBe(`system: <same>
tools: <same>
messages:
<last>
+ [assistant] hi
+ tool_use echo {}`);
});

it("prints the whole prompt when previous is longer than this request", () => {
  const shorter = request();
  const longer = request({
    messages: [
      ...shorter.messages,
      { role: "assistant", content: [{ type: "text", text: "hi" }] },
    ],
  });
  const rendered = renderLlmInput(shorter, longer);
  expect(rendered).not.toContain("<last>");
  expect(rendered).toContain("[user] hello");
});

it("prints only <last> when the message list did not grow", () => {
  const first = request();
  expect(renderLlmInput(first, first)).toBe(`system: <same>
tools: <same>
messages:
<last>`);
});

it("prints the whole prompt when the system head changes", () => {
  const first = request();
  const second = request({
    head: { system: "today is 2026-09-23", tools: [] },
    messages: [
      ...first.messages,
      { role: "assistant", content: [{ type: "text", text: "hi" }] },
    ],
  });
  const rendered = renderLlmInput(second, first);
  expect(rendered).not.toContain("<last>");
  expect(rendered).not.toContain("<same>");
  expect(rendered).toBe(`system: today is 2026-09-23
tools: []
messages:
[user] hello
[assistant] hi`);
});

it("prints the whole prompt when an earlier message is not a prefix", () => {
  const first = request();
  const second = request({
    messages: [
      { role: "user", content: [{ type: "text", text: "changed" }] },
      { role: "assistant", content: [{ type: "text", text: "hi" }] },
    ],
  });
  const rendered = renderLlmInput(second, first);
  expect(rendered).not.toContain("<last>");
  expect(rendered.startsWith("system: You are sage.")).toBe(true);
  expect(rendered).toContain("[user] changed");
});
