import { Buffer } from "node:buffer";
import { jsonFetch } from "@sage/core/testing";
import { expect, it } from "vitest";
import { jevScorer } from "../../src/scorer/jev.ts";
import {
  answerNouls,
  chunkOf,
  type FixtureCall,
  requestWith,
} from "./jev-fixtures.ts";

/** Most questions one request may carry: 4 fixed plus 24 `keep_<i>`. */
const MAX_QUESTIONS = 28;
/** Byte budget of one request body for a typical chunk. */
const MAX_BODY_BYTES = 20_000;

const GREP: FixtureCall = {
  id: "g1",
  tool: "grep",
  input: { pattern: "createHandler", path: "src" },
  output: Array.from(
    { length: 200 },
    (_, i) =>
      `src/components/module${i}/index.ts:${i * 3 + 1}:   export const handler${i} = createHandler("module-${i}");`,
  ).join("\n"),
};

const READ: FixtureCall = {
  id: "r1",
  tool: "read",
  input: { path: "src/server/router.ts" },
  output: Array.from({ length: 300 }, (_, i) => {
    if (i < 10) return `import { dep${i} } from "./deps/dep${i}.js";`;
    if (i % 25 === 0) {
      return `export function route${i}(request: Request, ctx: Context): Promise<Response> {`;
    }
    if (i % 25 === 24) return "}";
    return `  const value${i} = await ctx.lookup("key-${i}", request.headers);`;
  }).join("\n"),
};

const LS: FixtureCall = {
  id: "l1",
  tool: "ls",
  input: { path: "src/components" },
  output: Array.from({ length: 150 }, (_, i) =>
    i % 5 === 0 ? `group-${i}/` : `component-${i}.tsx`,
  ).join("\n"),
};

const VITEST: FixtureCall = {
  id: "v1",
  tool: "bash",
  input: { command: "pnpm vitest run" },
  output: [
    "[bash exit code 1]",
    "",
    " RUN  v5.0.1 /work/app",
    "",
    ...Array.from(
      { length: 80 },
      (_, i) => ` ✓ test/unit/feature-${i}.test.ts (6 tests) ${i + 3}ms`,
    ),
    " ❯ test/unit/parser.test.ts (4 tests | 1 failed) 21ms",
    "   × parses an empty document 9ms",
    "",
    "⎯⎯⎯⎯⎯⎯⎯ Failed Tests 1 ⎯⎯⎯⎯⎯⎯⎯",
    "",
    " FAIL  test/unit/parser.test.ts > parses an empty document",
    "AssertionError: expected undefined to deeply equal { nodes: [] }",
    "",
    "- Expected",
    "+ Received",
    "",
    "    at test/unit/parser.test.ts:42:18",
    "    at runTest (node_modules/vitest/dist/runner.js:310:9)",
    "",
    " Test Files  1 failed | 80 passed (81)",
    "      Tests  1 failed | 483 passed (484)",
    "   Start at  10:12:03",
    "   Duration  4.21s",
  ].join("\n"),
};

/** One captured request body for `call`, as bytes and question count. */
async function measure(call: FixtureCall) {
  const fake = jsonFetch(answerNouls({}));
  await jevScorer({ apiKey: "k", fetchImpl: fake.fetchImpl }).score({
    candidates: [chunkOf(call, 1, 4000)],
    prevRequest: requestWith(
      [{ ...call, prose: "Find where handlers are registered." }],
      {
        task: "fix the failing parser test",
        ledger:
          "Goal: fix the parser\nPlan:\n- read src/parser.ts\n- fix empty input",
      },
    ),
    query: "fix the failing parser test",
  });
  const body = fake.requests()[0]?.body;
  const questions =
    typeof body === "object" && body !== null && "questions" in body
      ? Object.keys(body.questions as object).length
      : 0;
  return {
    bytes: Buffer.byteLength(JSON.stringify(body), "utf8"),
    questions,
  };
}

it("keeps typical grep, read, ls, and vitest requests within 28 questions and 20KB", async () => {
  for (const call of [GREP, READ, LS, VITEST]) {
    const size = await measure(call);
    expect(size.questions).toBeGreaterThan(4);
    expect(size.questions).toBeLessThanOrEqual(MAX_QUESTIONS);
    expect(size.bytes).toBeLessThanOrEqual(MAX_BODY_BYTES);
  }
});
