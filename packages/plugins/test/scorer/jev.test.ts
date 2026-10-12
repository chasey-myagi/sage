import { jsonFetch } from "@sage/core/testing";
import { listModels } from "@sage/provider";
import { describe, expect, it } from "vitest";
import { chunkRequest } from "../../src/scorer/jev/state.ts";
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

const GREP_HITS: FixtureCall = {
  id: "t3",
  tool: "grep",
  input: { pattern: "parseConfig", path: "src" },
  prose: "Where is parseConfig called?",
  output: [
    "src/config.ts:3: export function parseConfig(text: string): Config {",
    "src/main.ts:10:   const config = parseConfig(raw);",
    "src/cli.ts:22:   return parseConfig(readFileSync(file, 'utf8'));",
  ].join("\n"),
};

const LISTING: FixtureCall = {
  id: "t4",
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
  it("posts one request per bash or grep chunk with the task, ledger, assistant prose, and that chunk's output", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}));
    await scorer.score({
      candidates: [chunkOf(TEST_RUN, 1), chunkOf(GREP_HITS, 3)],
      prevRequest: requestWith([TEST_RUN, GREP_HITS], {
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
    const grep = requests
      .map(sentBody)
      .find((body) => body.state.chunk.id === "seq:3#t3");
    expect(grep?.model).toBe("jev-latest");
    expect(grep?.state).toEqual({
      task: "fix parseConfig on empty input",
      ledger: "Goal: fix parseConfig\nPlan:\n- edit src/config.ts",
      recent_assistant: "Where is parseConfig called?",
      chunk: {
        id: "seq:3#t3",
        tool: "grep",
        input: '{"pattern":"parseConfig","path":"src"}',
        tokens: 400,
        head: [],
        tail: [],
        omitted_lines: 0,
        candidates: [
          "src/config.ts:3: export function parseConfig(text: string): Config {",
          "src/main.ts:10: const config = parseConfig(raw);",
          "src/cli.ts:22: return parseConfig(readFileSync(file, 'utf8'));",
        ],
      },
    });
    expect(JSON.stringify(grep)).not.toContain("pnpm test");
    const bash = requests
      .map(sentBody)
      .find((body) => body.state.chunk.id === "seq:1#t2");
    expect(bash?.state.recent_assistant).toBe("");
    // Every line of this output is a candidate, so no head or tail is sent.
    expect(bash?.state.chunk.head).toEqual([]);
    expect(bash?.state.chunk.candidates).toHaveLength(5);
  });

  it("asks reports_failure, locates_next_step, and one keep Noul per candidate line, each with true and false criteria", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}));
    await scorer.score({
      candidates: [chunkOf(GREP_HITS, 1)],
      prevRequest: requestWith([GREP_HITS]),
      query: "q",
    });
    const questions = sentBody(fake.requests()[0]).questions;
    expect(Object.keys(questions)).toEqual([
      "reports_failure",
      "locates_next_step",
      "keep_0",
      "keep_1",
      "keep_2",
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
    expect(questions.locates_next_step).toEqual({
      type: "noul",
      instructions:
        "Does the tool output in `chunk` name a file path, line number, test name, or error location that `recent_assistant` and `ledger` have not already mentioned?",
      criteria: {
        true: "The output shows at least one path, line, test name, or error location that does not appear in `recent_assistant` or `ledger`.",
        false:
          "The output only confirms success, repeats locations already named, or shows no location at all.",
      },
    });
    const statement =
      "`line` names a location or result `task` or `recent_assistant` needs.";
    expect(questions.keep_1).toEqual({
      type: "noul",
      instructions: {
        line: "src/main.ts:10: const config = parseConfig(raw);",
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

  it("asks a read chunk only shows_target_code, the question offline calibration sends", () => {
    const request = chunkRequest(
      chunkOf(CONFIG_READ, 1),
      CONFIG_READ.output,
      { task: "fix parseConfig", ledger: "", prose: "" },
      "read-probe",
    );
    expect(request?.set).toBe("read-probe");
    expect(request?.questions).toEqual({
      shows_target_code: {
        type: "noul",
        instructions:
          "Does `chunk` show the body of the function, class, or block that `task` or `ledger` names as the thing to change or verify?",
        criteria: {
          true: "The excerpt contains the definition or body lines of a symbol that `task` or `ledger` names.",
          false:
            "The excerpt shows other parts of the file, such as imports, headers, or unrelated helpers, or `task` and `ledger` name no symbol.",
        },
      },
    });
  });

  it("sends no request and gives no decision for read, ls, and other tools, leaving them to the rules", async () => {
    const other: FixtureCall = {
      id: "t5",
      tool: "mcp_search",
      input: { q: "x" },
      output: "error: upstream timed out",
    };
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [
        chunkOf(CONFIG_READ, 1),
        chunkOf(LISTING, 3),
        chunkOf(other, 5),
      ],
      prevRequest: requestWith([CONFIG_READ, LISTING, other]),
      query: "q",
    });
    expect(fake.requests()).toHaveLength(0);
    expect(result).toEqual({ decisions: [], requested: false });
  });

  it("sends no request for a bash command classifyBashCommand calls a reader, and triages runner and mixed commands", async () => {
    const reader: FixtureCall = {
      id: "t6",
      tool: "bash",
      input: { command: "cat src/config.ts" },
      output: "[bash exit code 0]\nexport function parseConfig() {}",
    };
    const mixed: FixtureCall = {
      ...TEST_RUN,
      id: "t7",
      input: { command: "cat a && pnpm test" },
    };
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [chunkOf(reader, 1), chunkOf(TEST_RUN, 3), chunkOf(mixed, 5)],
      prevRequest: requestWith([reader, TEST_RUN, mixed]),
      query: "q",
    });
    expect(
      fake.requests().map((request) => sentBody(request).state.chunk.id),
    ).toEqual(["seq:3#t2", "seq:5#t7"]);
    expect(result.decisions.map((decision) => decision.id)).toEqual([
      "seq:3#t2",
      "seq:5#t7",
    ]);
  });

  it("sends no request for a bash call without a string command", async () => {
    const odd: FixtureCall = {
      id: "t6",
      tool: "bash",
      input: { script: "pnpm test" },
      output: TEST_RUN.output,
    };
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [chunkOf(odd, 1)],
      prevRequest: requestWith([odd]),
      query: "q",
    });
    expect(fake.requests()).toHaveLength(0);
    expect(result).toEqual({ decisions: [], requested: false });
  });

  it("gives an isError read or ls no decision, leaving the compiler's tier, while an isError bash runner is full", async () => {
    const failedRead: FixtureCall = { ...CONFIG_READ, isError: true };
    const failedList: FixtureCall = { ...LISTING, isError: true };
    const failedRun: FixtureCall = { ...TEST_RUN, isError: true };
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [
        chunkOf(failedRead, 1),
        chunkOf(failedList, 3),
        chunkOf(failedRun, 5),
      ],
      prevRequest: requestWith([failedRead, failedList, failedRun]),
      query: "q",
    });
    expect(fake.requests()).toHaveLength(0);
    expect(result).toEqual({
      decisions: [
        { id: "seq:5#t2", tier: "full", reason: "scorer:jev:full:is_error" },
      ],
      requested: false,
    });
  });

  it("posts to baseUrl without a trailing slash and sends opts.model", async () => {
    const { fake, scorer } = scorerWith(answerNouls({}), {
      baseUrl: "https://jev.example/",
      model: "served-model",
    });
    await scorer.score({
      candidates: [chunkOf(TEST_RUN, 1)],
      prevRequest: requestWith([TEST_RUN]),
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
        reason: "scorer:jev:full:failure=0.40,locate=0.00,keep=0.90",
      },
    ]);
  });

  it("keeps a chunk full when locates_next_step reaches the full threshold", async () => {
    expect(
      await decide(GREP_HITS, { locates_next_step: 0.75, keep: [0.9] }),
    ).toEqual([
      {
        id: "seq:1#t3",
        tier: "full",
        reason: "scorer:jev:full:failure=0.00,locate=0.75,keep=0.90",
      },
    ]);
  });

  it("drops a chunk to none only when the failure, location, and every keep answer are below the low threshold", async () => {
    expect(
      await decide(GREP_HITS, {
        reports_failure: 0.19,
        locates_next_step: 0.1,
        keep: [0.19, 0.05, 0],
      }),
    ).toEqual([
      {
        id: "seq:1#t3",
        tier: "none",
        reason: "scorer:jev:none:failure=0.19,locate=0.10,keep=0.19",
      },
    ]);
    for (const nouls of [
      { reports_failure: 0.2 },
      { locates_next_step: 0.2 },
      { keep: [0, 0, 0.2] },
    ]) {
      const decisions = await decide(GREP_HITS, nouls);
      expect(decisions.map((decision) => decision.tier)).toEqual(["full"]);
    }
  });

  it("drops a chunk without candidate lines to none when failure and location are low", async () => {
    const quiet: FixtureCall = {
      id: "t8",
      tool: "bash",
      input: { command: "pnpm format" },
      output: "done in 2s",
    };
    expect(await decide(quiet, {})).toEqual([
      {
        id: "seq:1#t8",
        tier: "none",
        reason: "scorer:jev:none:failure=0.00,locate=0.00",
      },
    ]);
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
        reason: "scorer:jev:summary:failure=0.30,locate=0.00,keep=0.90",
      },
    ]);
  });

  it("keeps a chunk full when a keep line clears the low threshold but none reaches the keep threshold", async () => {
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
        reason: "scorer:jev:full:failure=0.00,locate=0.00,keep=0.99",
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
      GREP_HITS,
      { locates_next_step: 0.3, keep: [0.3] },
      { low: 0.35 },
    );
    expect(lenient.map((decision) => decision.tier)).toEqual(["none"]);
  });

  it("keeps an isError chunk full without asking Jev", async () => {
    const failed: FixtureCall = { ...TEST_RUN, isError: true };
    const { fake, scorer } = scorerWith(answerNouls({}));
    const result = await scorer.score({
      candidates: [chunkOf(failed, 1), chunkOf(GREP_HITS, 3)],
      prevRequest: requestWith([failed, GREP_HITS]),
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
      candidates: [chunkOf(GREP_HITS, 1)],
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
      candidates: [chunkOf(GREP_HITS, 1), stubbed],
      prevRequest: prev,
      query: "q",
    });
    expect(fake.requests()).toHaveLength(0);
    expect(result).toEqual({ decisions: [], requested: false });
  });
});
