import { expect, it } from "vitest";
import { systemReminder } from "../../src/claims/index.ts";
import { contextOf, runSession, stopScript } from "./session.ts";

it("submits a tail line claim and nothing else", async () => {
  const { store, sessionId } = await runSession({
    scripts: [stopScript("ok")],
  });
  const claims = await systemReminder({ text: "Stay on the task." })(
    contextOf(store, sessionId),
  );

  expect(claims).toEqual([
    {
      id: "system-reminder",
      priority: 20,
      placement: "tail",
      tiers: { line: "Stay on the task." },
    },
  ]);

  const overridden = await systemReminder({ text: "Other.", priority: 3 })(
    contextOf(store, sessionId),
  );
  expect(overridden[0]?.priority).toBe(3);
  expect(overridden[0]?.tiers).toEqual({ line: "Other." });
});
