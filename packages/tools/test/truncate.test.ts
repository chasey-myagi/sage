import { describe, expect, it } from "vitest";
import { truncate } from "../src/truncate.ts";

describe("truncate", () => {
  it("keeps whole lines and reports omitted lines and bytes", () => {
    const result = truncate("a\nb\nc", { maxLines: 2, maxBytes: 100 });
    expect(result.text).toBe("a\nb");
    expect(result.omitted).toEqual({ lines: 1, bytes: 2 });
  });

  it("does not return a partial line when the byte budget runs out", () => {
    const result = truncate("abcd\nEFGH", { maxLines: 10, maxBytes: 6 });
    expect(result.text).toBe("abcd");
    expect(result.text).not.toContain("EF");
    expect(result.omitted.lines).toBe(1);
  });

  it("returns no text when the first line itself exceeds the byte budget", () => {
    const result = truncate("abcdef", { maxLines: 10, maxBytes: 3 });
    expect(result.text).toBe("");
    expect(result.omitted).toEqual({ lines: 1, bytes: 6 });
  });

  it("returns the input unchanged when both limits fit", () => {
    const result = truncate("a\nb", { maxLines: 5, maxBytes: 50 });
    expect(result).toEqual({
      text: "a\nb",
      omitted: { lines: 0, bytes: 0 },
    });
  });
});
