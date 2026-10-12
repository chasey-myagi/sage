import { jsonFetch } from "@sage/core/testing";
import { listModels } from "@sage/provider";
import { describe, expect, it } from "vitest";
import { jevScorer } from "../../src/scorer/jev.ts";
import {
  answerNouls,
  chunkOf,
  type FixtureCall,
  requestWith,
  sentBody,
} from "./jev-fixtures.ts";

const SOURCE = "https://typesafe.ai/blog/introducing-system-one-models-and-jev";

const CONFIG_READ: FixtureCall = {
  id: "t1",
  tool: "read",
  input: { path: "src/config.ts" },
  prose: "I will look for parseConfig in src/config.ts.",
  output: [
    "import { readFileSync } from 'node:fs';",
    "",
    "export function parseConfig(text: string): Config {",
    "  return JSON.parse(text);",
    "}",
    "",
    "export class ConfigError extends Error {}",
  ].join("\n"),
};

const TEST_RUN: FixtureCall = {
  id: "t2",
  tool: "bash",
  input: { command: "pnpm test" },
  output: [
    "[bash exit code 1]",
    " FAIL  test/config.test.ts > parses empty input",
    "AssertionError: expected undefined to equal {}",
    "    at test/config.test.ts:12:5",
    " Tests  1 failed | 22 passed (23)",
  ].join("\n"),
};

const LISTING: FixtureCall = {
  id: "t3",
  tool: "ls",
  input: { path: "src" },
  output: ["config.ts", "index.ts", "util/"].join("\n"),
};

function scorerWith(
  respond: Parameters<typeof jsonFetch>[0],
  extra: Partial<Parameters<typeof jevScorer>[0]> = {},
) {
  const fake = jsonFetch(respond);
  return {
    fake,
    scorer: jevScorer({
      apiKey: "secret",
      fetchImpl: fake.fetchImpl,
      ...extra,
    }),
  };
}

it("lists jev-latest on the typesafe provider for pricing only", () => {
  expect(listModels("typesafe")).toEqual([
    {
      provider: "typesafe",
      model: "jev-latest",
      contextWindow: 32000,
      pricing: {
        inputPerMTok: 0.042,
        outputPerMTok: 0,
        cacheWritePerMTok: 0,
        cacheReadPerMTok: 0,
        source: SOURCE,
      },
    },
  ]);
});

describe("request", () => {
  it("posts one request per chunk with the task, ledger, assistant prose, and that chunk's output", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}));
    await scorer.score({
      candidates: [chunkOf(CONFIG_READ, 1), chunkOf(TEST_RUN, 3)],
      prevRequest: requestWith([CONFIG_READ, TEST_RUN], {
        task: "fix parseConfig on empty input",
        ledger: "Goal: fix parseConfig\nPlan:\n- edit src/config.ts",
      }),
      query: "fix parseConfig on empty input",
    });

    const requests = fake.requests();
    expect(requests).toHaveLength(2);
    for (const request of requests) {
      expect(request.url).toBe("https://api.typesafe.ai/v1/systemone");
      expect(request.method).toBe("POST");
      expect(request.headers).toEqual({
        Authorization: "Bearer secret",
        "Content-Type": "application/json",
      });
    }
    const read = requests
      .map(sentBody)
      .find((body) => body.state.chunk.id === "seq:1#t1");
    expect(read?.model).toBe("jev-latest");
    expect(read?.state).toEqual({
      task: "fix parseConfig on empty input",
      ledger: "Goal: fix parseConfig\nPlan:\n- edit src/config.ts",
      recent_assistant: "I will look for parseConfig in src/config.ts.",
      chunk: {
        id: "seq:1#t1",
        tool: "read",
        input: '{"path":"src/config.ts"}',
        tokens: 400,
        head: CONFIG_READ.output.split("\n"),
        tail: [],
        omitted_lines: 0,
        candidates: [
          "L3: export function parseConfig(text: string): Config {",
          "L7: export class ConfigError extends Error {}",
        ],
      },
    });
    expect(JSON.stringify(read)).not.toContain("pnpm test");
    const bash = requests
      .map(sentBody)
      .find((body) => body.state.chunk.id === "seq:3#t2");
    expect(bash?.state.recent_assistant).toBe("");
    // Every line of this output is a candidate, so no head or tail is sent.
    expect(bash?.state.chunk.head).toEqual([]);
    expect(bash?.state.chunk.candidates).toHaveLength(5);
  });

  it("asks four fixed Nouls plus one keep Noul per candidate line, each with true and false criteria", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}));
    await scorer.score({
      candidates: [chunkOf(CONFIG_READ, 1)],
      prevRequest: requestWith([CONFIG_READ]),
      query: "q",
    });
    const questions = sentBody(fake.requests()[0]).questions;
    expect(Object.keys(questions)).toEqual([
      "reports_failure",
      "is_edit_target",
      "answers_lookup",
      "is_broad_listing",
      "keep_0",
      "keep_1",
    ]);
    expect(Object.values(questions).every((q) => q.type === "noul")).toBe(true);
    expect(questions.reports_failure).toEqual({
      type: "noul",
      instructions:
        "Does the tool output in `chunk` report that something failed?",
      criteria: {
        true: "The output reports an error, an exception, a non-zero exit code, a failing test, or a failed build or check.",
        false:
          "The output reports success or only shows data; error words inside file contents being read do not count.",
      },
    });
    expect(questions.is_edit_target).toEqual({
      type: "noul",
      instructions:
        "Is the file at the path in `chunk.input` a file that `recent_assistant` or `ledger` says will be edited?",
      criteria: {
        true: "`recent_assistant` or `ledger` says the file at that path will be changed, fixed, or written.",
        false:
          "No edit to that file is mentioned, or `chunk.input` names no single file path.",
      },
    });
    expect(questions.answers_lookup).toEqual({
      type: "noul",
      instructions:
        "Does the tool output in `chunk` contain the specific symbol, line, or value that `recent_assistant` says it is looking for?",
      criteria: {
        true: "`recent_assistant` names a specific thing to find, such as a definition, a call site, a setting, or an error line, and the output in `chunk` shows it.",
        false:
          "`recent_assistant` does not name a specific thing to find, or the output in `chunk` does not show it.",
      },
    });
    expect(questions.is_broad_listing).toEqual({
      type: "noul",
      instructions:
        "Is the tool output in `chunk` a directory listing or a broad search result?",
      criteria: {
        true: "The output is a list of file or directory names, or search hits spread over many files.",
        false:
          "The output is file contents, command or test output, a diff, or a few search hits.",
      },
    });
    const statement =
      "`line` names a location or result `task` or `recent_assistant` needs.";
    expect(questions.keep_1).toEqual({
      type: "noul",
      instructions: {
        line: "L7: export class ConfigError extends Error {}",
        statement,
      },
      criteria: {
        true: statement,
        false: "`line` is boilerplate or unrelated to `task`.",
      },
    });
    const keepBytes = JSON.stringify({
      ...questions.keep_1,
      instructions: { line: "", statement },
    }).length;
    expect(keepBytes).toBeLessThanOrEqual(280);
    expect(statement.length).toBeLessThanOrEqual(80);
  });

  it("posts to baseUrl without a trailing slash and sends opts.model", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}), {
      baseUrl: "https://jev.example/",
      model: "served-model",
    });
    await scorer.score({
      candidates: [chunkOf(LISTING, 1)],
      prevRequest: requestWith([LISTING]),
      query: "q",
    });
    expect(fake.requests()[0]?.url).toBe("https://jev.example/v1/systemone");
    expect(sentBody(fake.requests()[0]).model).toBe("served-model");
  });
});

describe("tiers", () => {
  async function decide(
    call: FixtureCall,
    nouls: Parameters<typeof answerNouls>[0][string],
    thresholds?: Parameters<typeof jevScorer>[0]["thresholds"],
  ) {
    const chunk = chunkOf(call, 1);
    const { scorer } = scorerWith(answerNouls({ [chunk.id]: nouls }), {
      ...(thresholds ? { thresholds } : {}),
    });
    const result = await scorer.score({
      candidates: [chunk],
      prevRequest: requestWith([call]),
      query: "fix parseConfig",
    });
    expect(result.fallback).toBeUndefined();
    return result.decisions;
  }

  it("keeps a chunk full when reports_failure reaches the full threshold", async () => {
    expect(
      await decide(TEST_RUN, { reports_failure: 0.4, keep: [0.9, 0.9] }),
    ).toEqual([
      {
        id: "seq:1#t2",
        tier: "full",
        reason:
          "scorer:jev:full:failure=0.40,edit=0.00,lookup=0.00,broad=0.00,keep=0.90",
      },
    ]);
  });

  it("keeps a chunk full when is_edit_target reaches the full threshold", async () => {
    const decisions = await decide(CONFIG_READ, {
      is_edit_target: 0.75,
      is_broad_listing: 0.95,
    });
    expect(decisions).toEqual([
      {
        id: "seq:1#t1",
        tier: "full",
        reason:
          "scorer:jev:full:failure=0.00,edit=0.75,lookup=0.00,broad=0.95,keep=0.00",
      },
    ]);
  });

  it("drops a broad listing to none only when every other judgment is below the low threshold", async () => {
    expect(
      await decide(LISTING, { is_broad_listing: 0.8, answers_lookup: 0.19 }),
    ).toEqual([
      {
        id: "seq:1#t3",
        tier: "none",
        reason:
          "scorer:jev:none:failure=0.00,edit=0.00,lookup=0.19,broad=0.80,keep=0.00",
      },
    ]);
    const looked = await decide(LISTING, {
      is_broad_listing: 0.95,
      answers_lookup: 0.2,
    });
    expect(looked.map((decision) => decision.tier)).toEqual(["full"]);
  });

  it("summarizes with the three highest keep lines above the keep threshold, in output order", async () => {
    const decisions = await decide(TEST_RUN, {
      reports_failure: 0.3,
      keep: [0.6, 0.9, 0.2, 0.7, 0.8],
    });
    expect(decisions).toEqual([
      {
        id: "seq:1#t2",
        tier: "summary",
        summary:
          "FAIL test/config.test.ts > parses empty input; at test/config.test.ts:12:5; Tests 1 failed | 22 passed (23)",
        reason:
          "scorer:jev:summary:failure=0.30,edit=0.00,lookup=0.00,broad=0.00,keep=0.90",
      },
    ]);
  });

  it("keeps a chunk full when no keep line reaches the keep threshold", async () => {
    const decisions = await decide(TEST_RUN, { keep: [0.49, 0.1] });
    expect(decisions.map((decision) => decision.tier)).toEqual(["full"]);
  });

  it("keeps a chunk full when the chosen lines do not form a locatable summary", async () => {
    const short: FixtureCall = {
      id: "t9",
      tool: "bash",
      input: { command: "make" },
      output: "Error: boom",
    };
    const decisions = await decide(short, { keep: [0.99] });
    expect(decisions).toEqual([
      {
        id: "seq:1#t9",
        tier: "full",
        reason:
          "scorer:jev:full:failure=0.00,edit=0.00,lookup=0.00,broad=0.00,keep=0.99",
      },
    ]);
  });

  it("reads each threshold from opts.thresholds when given", async () => {
    const strict = await decide(
      TEST_RUN,
      { reports_failure: 0.4, keep: [0.6, 0.9] },
      { full: 0.5, keep: 0.95 },
    );
    expect(strict.map((decision) => decision.tier)).toEqual(["full"]);
    const lenient = await decide(
      LISTING,
      { is_broad_listing: 0.6, answers_lookup: 0.3 },
      { none: 0.5, low: 0.35 },
    );
    expect(lenient.map((decision) => decision.tier)).toEqual(["none"]);
  });

  it("keeps a chunk full when it holds what the assistant is looking for, instead of cutting it to a summary", async () => {
    const decisions = await decide(CONFIG_READ, {
      answers_lookup: 0.4,
      keep: [0.9, 0.9],
    });
    expect(decisions).toEqual([
      {
        id: "seq:1#t1",
        tier: "full",
        reason:
          "scorer:jev:full:failure=0.00,edit=0.00,lookup=0.40,broad=0.00,keep=0.90",
      },
    ]);
  });

  it("keeps an isError chunk full without asking Jev", async () => {
    const failed: FixtureCall = { ...TEST_RUN, isError: true };
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [chunkOf(failed, 1), chunkOf(LISTING, 3)],
      prevRequest: requestWith([failed, LISTING]),
      query: "q",
    });
    expect(fake.requests().map((r) => sentBody(r).state.chunk.id)).toEqual([
      "seq:3#t3",
    ]);
    expect(result.decisions).toContainEqual({
      id: "seq:1#t2",
      tier: "full",
      reason: "scorer:jev:full:is_error",
    });
    expect(result.decisions).toHaveLength(2);

    const alone = scorerWith(answerNouls({}));
    const only = await alone.scorer.score({
      candidates: [chunkOf(failed, 1)],
      prevRequest: requestWith([failed]),
      query: "q",
    });
    expect(alone.fake.requests()).toHaveLength(0);
    expect(only).toEqual({
      decisions: [
        { id: "seq:1#t2", tier: "full", reason: "scorer:jev:full:is_error" },
      ],
      requested: false,
    });
  });
});

describe("sources", () => {
  it("falls back without fetching when there is no previous request", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [chunkOf(CONFIG_READ, 1)],
      prevRequest: undefined,
      query: "q",
    });
    expect(result).toEqual({
      decisions: [],
      fallback: "no-previous-request",
      requested: false,
    });
    expect(fake.requests()).toHaveLength(0);
  });

  it("skips a chunk whose result is missing or already the stub, so it stays as the compiler left it", async () => {
    const stubbed = chunkOf(TEST_RUN, 3);
    const prev = requestWith([
      {
        ...TEST_RUN,
        output: `[#${stubbed.id} · bash · {"command":"pnpm test"} · 0.4k tokens · omitted · expand("${stubbed.id}") to retrieve]`,
      },
    ]);
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [chunkOf(CONFIG_READ, 1), stubbed],
      prevRequest: prev,
      query: "q",
    });
    expect(fake.requests()).toHaveLength(0);
    expect(result).toEqual({ decisions: [], requested: false });
  });
});
