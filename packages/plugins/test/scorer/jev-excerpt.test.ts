import { jsonFetch } from "@sage/core/testing";
import { expect, it } from "vitest";
import { jevScorer } from "../../src/scorer/jev.ts";
import {
  answerNouls,
  chunkOf,
  type FixtureCall,
  requestWith,
  type SentBody,
  sentBody,
} from "./jev-fixtures.ts";

/** The chunk state jev received for `call`. */
async function sentChunk(
  call: FixtureCall,
): Promise<SentBody["state"]["chunk"]> {
  return (await sentFor(call)).state.chunk;
}

async function sentFor(call: FixtureCall): Promise<SentBody> {
  const fake = jsonFetch(answerNouls({}));
  await jevScorer({ apiKey: "k", fetchImpl: fake.fetchImpl }).score({
    candidates: [chunkOf(call, 1)],
    prevRequest: requestWith([call]),
    query: "q",
  });
  return sentBody(fake.requests()[0]);
}

function call(tool: string, input: unknown, lines: string[]): FixtureCall {
  return { id: "t1", tool, input, output: lines.join("\n") };
}

it("offers read signature and export lines numbered from the input offset", async () => {
  const chunk = await sentChunk(
    call("read", { path: "src/a.py", offset: 40 }, [
      "import os",
      "def load(path):",
      "    return open(path)",
      "class Loader:",
      "export const DEFAULT = 1;",
      "[truncated: showing lines 40-44 of 90; call read with offset=45 to continue]",
    ]),
  );
  expect(chunk.candidates).toEqual([
    "L41: def load(path):",
    "L43: class Loader:",
    "L44: export const DEFAULT = 1;",
  ]);
});

it("offers every grep hit and drops the truncation footer", async () => {
  const chunk = await sentChunk(
    call("grep", { pattern: "parseConfig" }, [
      "src/config.ts:3: export function parseConfig(text: string) {",
      "src/main.ts:10:   const config = parseConfig(raw);",
      "[truncated: showing 2 of 9 matches; narrow the path or raise max_results]",
    ]),
  );
  expect(chunk.candidates).toEqual([
    "src/config.ts:3: export function parseConfig(text: string) {",
    "src/main.ts:10: const config = parseConfig(raw);",
  ]);
});

it("offers bash failures, stack frames, path:line refs, test summaries, and diff hunk headers", async () => {
  const chunk = await sentChunk(
    call("bash", { command: "pnpm test" }, [
      "[bash exit code 1]",
      "> vitest run",
      "",
      "TypeError: cannot read properties of undefined",
      "    at parse (src/config.ts:4:10)",
      '  File "tool.py", line 3, in main',
      "src/util.ts:8:1 - warning: unused import",
      "just some log output",
      "Tests  2 failed | 21 passed (23)",
      "@@ -10,6 +10,8 @@ export function parse",
      "diff --git a/src/config.ts b/src/config.ts",
    ]),
  );
  expect(chunk.candidates).toEqual([
    "[bash exit code 1]",
    "TypeError: cannot read properties of undefined",
    "at parse (src/config.ts:4:10)",
    'File "tool.py", line 3, in main',
    "src/util.ts:8:1 - warning: unused import",
    "Tests 2 failed | 21 passed (23)",
    "@@ -10,6 +10,8 @@ export function parse",
    "diff --git a/src/config.ts b/src/config.ts",
  ]);
});

it("offers every ls and find entry and drops the truncation footer", async () => {
  for (const tool of ["ls", "find"]) {
    const chunk = await sentChunk(
      call(tool, { path: "src" }, [
        "a.ts",
        "util/",
        "[truncated: showing 2 of 400 entries]",
      ]),
    );
    expect(chunk.candidates).toEqual(["a.ts", "util/"]);
  }
});

it("offers failure, location, signature, and summary lines for a tool it does not know", async () => {
  const chunk = await sentChunk(
    call("mcp_search", { q: "x" }, [
      "hello",
      "error: upstream timed out",
      "docs/guide.md:12: see the install section",
      "export function search(q) {",
      "3 passed",
    ]),
  );
  expect(chunk.candidates).toEqual([
    "error: upstream timed out",
    "docs/guide.md:12: see the install section",
    "export function search(q) {",
    "3 passed",
  ]);
});

it("keeps the first and last 30 candidates, each cut to 200 characters", async () => {
  const lines = Array.from(
    { length: 100 },
    (_, i) => `src/f${i}.ts:${i + 1}: ${"x".repeat(300)}`,
  );
  const chunk = await sentChunk(call("grep", { pattern: "x" }, lines));
  expect(chunk.candidates).toHaveLength(60);
  expect(chunk.candidates[0]?.startsWith("src/f0.ts:1: ")).toBe(true);
  expect(chunk.candidates[29]?.startsWith("src/f29.ts:30: ")).toBe(true);
  expect(chunk.candidates[30]?.startsWith("src/f70.ts:71: ")).toBe(true);
  expect(chunk.candidates[59]?.startsWith("src/f99.ts:100: ")).toBe(true);
  expect(chunk.candidates.every((line) => line.length <= 200)).toBe(true);
});

it("sends the first 40 and last 20 lines and counts the lines in between", async () => {
  const lines = Array.from({ length: 100 }, (_, i) => `line ${i}`);
  const chunk = await sentChunk(call("bash", { command: "seq" }, lines));
  expect(chunk.head).toEqual(lines.slice(0, 40));
  expect(chunk.tail).toEqual(lines.slice(80));
  expect(chunk.omitted_lines).toBe(40);
});

it("cuts head and tail until the state plus the longest question fits 32k estimated tokens", async () => {
  const lines = Array.from(
    { length: 300 },
    (_, i) => `error ${i}: ${"错".repeat(400)}`,
  );
  const body = await sentFor(call("bash", { command: "build" }, lines));
  const chunk = body.state.chunk;
  expect(chunk.head.length).toBeGreaterThan(0);
  expect(chunk.head.length).toBeLessThan(40);
  expect(chunk.tail.length).toBeLessThan(20);
  expect(chunk.omitted_lines).toBe(300 - chunk.head.length - chunk.tail.length);
  expect(chunk.candidates).toHaveLength(60);
  // Jev counts the state plus the longest question against 32k tokens and
  // the state plus every question against 64k. 2.5 UTF-8 bytes per token
  // is the scorer's conservative estimate.
  const bytes = (value: unknown) =>
    Buffer.byteLength(JSON.stringify(value), "utf8");
  const questionBytes = Object.values(body.questions).map(bytes);
  const stateBytes = bytes(body.state);
  expect(stateBytes + Math.max(...questionBytes)).toBeLessThanOrEqual(
    32_000 * 2.5,
  );
  expect(
    stateBytes + questionBytes.reduce((sum, n) => sum + n, 0),
  ).toBeLessThanOrEqual(64_000 * 2.5);
});

it("cuts long head and tail lines to 400 characters and the input to 1000", async () => {
  const chunk = await sentChunk(
    call("bash", { command: "y".repeat(2000) }, ["z".repeat(1000)]),
  );
  expect(chunk.head[0]?.length).toBe(400);
  expect(chunk.input.length).toBe(1000);
});
