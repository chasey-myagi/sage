import { describe, expect, it } from "vitest";
import { resolvePreset } from "../src/index.ts";

const table = [
  {
    name: "openai",
    baseUrl: "https://api.openai.com/v1",
    envKey: "OPENAI_API_KEY",
    api: "openai-chat",
  },
  {
    name: "deepseek",
    baseUrl: "https://api.deepseek.com",
    envKey: "DEEPSEEK_API_KEY",
    api: "openai-chat",
  },
  {
    name: "dashscope",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    envKey: "DASHSCOPE_API_KEY",
    api: "openai-chat",
  },
  {
    name: "moonshot",
    baseUrl: "https://api.moonshot.cn/v1",
    envKey: "MOONSHOT_API_KEY",
    api: "openai-chat",
  },
  {
    name: "anthropic",
    baseUrl: "https://api.anthropic.com",
    envKey: "ANTHROPIC_API_KEY",
    api: "anthropic-messages",
  },
] as const;

describe("provider presets", () => {
  it("resolves name to base URL, env key, and api", () => {
    for (const row of table) {
      expect(resolvePreset(row.name)).toEqual(row);
    }
    expect(resolvePreset("missing")).toBeUndefined();
  });
});
