import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { SPILL_LIMIT_BYTES, spill } from "../src/spill.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
});

describe("spill", () => {
  it("returns short output unchanged and does not write a file", () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-spill-"));
    const file = path.join(dir, "out.txt");
    const result = spill("hello", file);
    expect(result).toEqual({ text: "hello", saved: false });
    expect(() => readFileSync(file, "utf8")).toThrow();
  });

  it("keeps a 20000 byte head and 5000 byte tail with the notice inside the cap", () => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-spill-"));
    const file = path.join(dir, "toolu_test.txt");
    const output = "x".repeat(30_001);
    const result = spill(output, file);
    expect(result.saved).toBe(true);
    expect(Buffer.byteLength(result.text)).toBeLessThanOrEqual(
      SPILL_LIMIT_BYTES,
    );
    expect(result.text.startsWith("x".repeat(20_000))).toBe(true);
    expect(result.text.endsWith("x".repeat(5_000))).toBe(true);
    expect(result.text).toContain(
      `[… omitted 5001 bytes; full output saved to ${file}; read it with read(path, offset=…) …]`,
    );
    expect(readFileSync(file, "utf8")).toBe(output);
  });
});
