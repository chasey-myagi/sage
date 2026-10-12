import { expect, it } from "vitest";
import { startDeadline } from "../../src/scorer/deadline.ts";

it("accepts a fractional or out-of-range timeout instead of throwing, and still times out", async () => {
  for (const timeoutMs of [1.5, -5, Number.NaN, 2 ** 32]) {
    expect(() => startDeadline(timeoutMs, undefined)).not.toThrow();
  }
  const deadline = startDeadline(1.5, undefined);
  await deadline.aborted;
  expect(deadline.cause()).toBe("timeout");
});
