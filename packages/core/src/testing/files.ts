/**
 * Committed expected-output files.
 *
 * Contract: a missing file is created. `UPDATE_EXPECTED=1` rewrites it.
 * Otherwise the bytes must match, and a mismatch throws with a line diff.
 *
 * Intentionally not in the Log: expected files and diffs. They are the test
 * oracle, not a second copy of a session.
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

export function expectFile(file: string, actual: string): void {
  const update = process.env.UPDATE_EXPECTED === "1";
  if (update || !existsSync(file)) {
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, actual);
    return;
  }
  const expected = readFileSync(file, "utf8");
  if (expected === actual) return;
  throw new Error(
    `Expected file differs: ${file}\n${lineDiff(expected, actual)}`,
  );
}

function lineDiff(expected: string, actual: string): string {
  const before = expected.split("\n");
  const after = actual.split("\n");
  const lines = ["--- expected", "+++ actual"];
  const max = Math.max(before.length, after.length);
  for (let i = 0; i < max; i += 1) {
    if (before[i] === after[i]) continue;
    if (i < before.length) lines.push(`-${before[i] ?? ""}`);
    if (i < after.length) lines.push(`+${after[i] ?? ""}`);
  }
  return lines.join("\n");
}
