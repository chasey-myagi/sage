import { Buffer } from "node:buffer";
import { expect, it } from "vitest";
import { questionSetFor } from "../../src/scorer/jev/questions.ts";
import { chunkRequest, requestBody } from "../../src/scorer/jev/state.ts";
import { chunkOf, type FixtureCall } from "./jev-fixtures.ts";

/** Most questions one triage request may carry: 2 fixed plus 24 `keep_<i>`. */
const MAX_QUESTIONS = 26;
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

/** One request body for `call` as the scorer would build it, as bytes and question count. */
function measure(call: FixtureCall) {
  const chunk = chunkOf(call, 1, 4000);
  const set = questionSetFor(chunk);
  if (set === undefined) throw new Error(`${call.tool} is not asked`);
  const request = chunkRequest(
    chunk,
    call.output,
    {
      task: "fix the failing parser test",
      ledger:
        "Goal: fix the parser\nPlan:\n- read src/parser.ts\n- fix empty input",
      prose: "Find where handlers are registered.",
    },
    set,
  );
  if (!request) throw new Error(`${call.tool} request does not fit`);
  const body = requestBody(request, "jev-latest");
  return {
    bytes: Buffer.byteLength(body, "utf8"),
    questions: Object.keys(request.questions).length,
  };
}

it("keeps typical grep and vitest triage requests within 26 questions and 20KB", () => {
  for (const call of [GREP, VITEST]) {
    const size = measure(call);
    expect(size.questions).toBeGreaterThan(2);
    expect(size.questions).toBeLessThanOrEqual(MAX_QUESTIONS);
    expect(size.bytes).toBeLessThanOrEqual(MAX_BODY_BYTES);
  }
});

it("keeps a typical read probe to one question within 20KB", () => {
  expect(measure(READ).questions).toBe(1);
  expect(measure(READ).bytes).toBeLessThanOrEqual(MAX_BODY_BYTES);
});
