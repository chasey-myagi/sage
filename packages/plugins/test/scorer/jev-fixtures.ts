import type { JsonFetchReply, JsonFetchRequest } from "@sage/core/testing";
import type { Message, Request } from "@sage/provider/contract";
import type { Chunk } from "../../src/compiler/chunks.ts";

/** One tool call of a fixture request and the output it returned. */
export type FixtureCall = {
  id: string;
  tool: string;
  input: unknown;
  output: string;
  isError?: boolean;
  prose?: string;
};

/** The chunk the compiler would index for `call` at assistant seq `seq`. */
export function chunkOf(call: FixtureCall, seq: number, tokens = 400): Chunk {
  return {
    id: `seq:${seq}#${call.id}`,
    kind: "tool",
    messageIndex: 0,
    toolUseId: call.id,
    toolName: call.tool,
    input: call.input,
    isError: call.isError ?? false,
    tokens,
    accesses: [],
  };
}

/**
 * A request whose messages are the task, an optional ledger claim, and
 * one assistant tool call plus its result per `calls` entry.
 */
export function requestWith(
  calls: readonly FixtureCall[],
  opts: { task?: string; ledger?: string } = {},
): Request {
  const messages: Message[] = [
    {
      role: "user",
      content: [{ type: "text", text: opts.task ?? "find the bug" }],
    },
  ];
  for (const call of calls) {
    messages.push({
      role: "assistant",
      content: [
        ...(call.prose ? [{ type: "text" as const, text: call.prose }] : []),
        {
          type: "tool_use",
          id: call.id,
          name: call.tool,
          input: call.input,
          rawInput: JSON.stringify(call.input),
        },
      ],
    });
    messages.push({
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: call.id,
          content: call.output,
          isError: call.isError ?? false,
        },
      ],
    });
  }
  if (opts.ledger !== undefined) {
    const last = messages.at(-1);
    last?.content.push({
      type: "text",
      text: `<claim id="progress-ledger" tier="full">\n${opts.ledger}\n</claim>`,
    });
  }
  return {
    model: { provider: "anthropic", model: "claude-test" },
    head: { system: "You are sage.", tools: [] },
    messages,
    cacheBreakpoints: ["head"],
    sampling: {},
  };
}

/** Noul values one fixture chunk answers with; absent judgments are 0. */
export type FixtureNouls = {
  reports_failure?: number;
  is_edit_target?: number;
  answers_lookup?: number;
  is_broad_listing?: number;
  /** keep_<i> by index; a missing index answers 0. */
  keep?: readonly number[];
};

/** The parts of a captured jev request body the tests read. */
export type SentBody = {
  model?: string;
  state: {
    task: string;
    ledger: string;
    recent_assistant: string;
    chunk: {
      id: string;
      tool: string;
      input: string;
      is_error: boolean;
      tokens: number;
      head: string[];
      tail: string[];
      omitted_lines: number;
      candidates: string[];
    };
  };
  questions: Record<
    string,
    {
      type: string;
      instructions: unknown;
      criteria: { true: string; false: string };
    }
  >;
};

/**
 * Reads a captured request body as a jev body. Throws when it is not
 * one, so a malformed request fails the test that sent it.
 */
export function sentBody(request: JsonFetchRequest | undefined): SentBody {
  const body = request?.body;
  if (typeof body !== "object" || body === null || !("state" in body)) {
    throw new Error(`not a jev request body: ${JSON.stringify(body)}`);
  }
  // The guard above proves the body is an object with `state`; the test
  // then reads the fields it asserts on.
  return body as SentBody;
}

/**
 * Answers every question of a jev request with the Noul value `byChunk`
 * gives that request's chunk id, plus `usage`.
 */
export function answerNouls(
  byChunk: Record<string, FixtureNouls>,
  usage: { input_tokens: number; output_tokens: number } = {
    input_tokens: 10,
    output_tokens: 2,
  },
): (request: JsonFetchRequest) => JsonFetchReply {
  return (request) => {
    const body = sentBody(request);
    const values = byChunk[body.state.chunk.id] ?? {};
    const answers: Record<string, { type: "noul"; noul: number }> = {};
    for (const key of Object.keys(body.questions)) {
      answers[key] = { type: "noul", noul: valueFor(values, key) };
    }
    return { json: { model: "jev-1.13.0", answers, usage } };
  };
}

function valueFor(values: FixtureNouls, key: string): number {
  if (key.startsWith("keep_")) {
    return values.keep?.[Number(key.slice("keep_".length))] ?? 0;
  }
  switch (key) {
    case "reports_failure":
      return values.reports_failure ?? 0;
    case "is_edit_target":
      return values.is_edit_target ?? 0;
    case "answers_lookup":
      return values.answers_lookup ?? 0;
    case "is_broad_listing":
      return values.is_broad_listing ?? 0;
    default:
      throw new Error(`unexpected question id ${key}`);
  }
}
