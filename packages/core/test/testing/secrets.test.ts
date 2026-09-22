import { expect, it } from "vitest";
import { stripSecrets } from "../../src/testing/index.ts";

it("deletes *_API_KEY names and returns them", () => {
  const env: NodeJS.ProcessEnv = {
    ANTHROPIC_API_KEY: "sk-ant-test",
    OPENAI_API_KEY: "sk-openai-test",
    PATH: "/usr/bin",
    HOME: "/tmp",
  };
  const removed = stripSecrets(env);
  expect(removed.sort()).toEqual(["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
  expect(env.ANTHROPIC_API_KEY).toBeUndefined();
  expect(env.OPENAI_API_KEY).toBeUndefined();
  expect(env.PATH).toBe("/usr/bin");
  expect(env.HOME).toBe("/tmp");
});

it("leaves the test process with no *_API_KEY", () => {
  const leaked = Object.keys(process.env).filter((key) =>
    key.endsWith("_API_KEY"),
  );
  expect(leaked).toEqual([]);
  expect(process.env.ANTHROPIC_API_KEY).toBeUndefined();
});

it("fails the test process when fetch is called", async () => {
  await expect(fetch("https://api.anthropic.com/v1/messages")).rejects.toThrow(
    /network disabled/,
  );
});
