import type { Access } from "@sage/core";
import { type CatalogEntry, listModels } from "@sage/provider";
import { describe, expect, it } from "vitest";
import type { EpochDecision } from "../../src/compiler/decide.ts";
import {
  composePolicies,
  costTierPolicy,
  sensitivePolicy,
  withSensitiveFilter,
} from "../../src/routing/policy.ts";
import type { RouteInput, RoutePolicy } from "../../src/routing/types.ts";

const ENV = /(^|\/)\.env$|\/secrets\//;

const LADDER = [
  { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
  { provider: "anthropic", model: "claude-sonnet-5" },
  { provider: "openai", model: "gpt-5.4" },
] as const;

function decision(): EpochDecision {
  return {
    rebuild: true,
    oldTokens: 100,
    newTokens: 40,
    remainingTurns: 10,
    readRate: 0.1,
    writeRate: 1.25,
    savingsPerTurn: 6,
    extraCost: 40,
    paybackTurns: 7,
    reason: "payback:7<=10",
  };
}

function both(): CatalogEntry[] {
  return [...listModels("anthropic"), ...listModels("openai")];
}

function chunk(
  tier: "none" | "line" | "full",
  key?: string,
): RouteInput["viewChunks"][number] {
  const accesses: Access[] = key ? [{ key, mode: "read" }] : [];
  return { id: `c-${tier}-${key ?? "none"}`, tier, reason: "kept", accesses };
}

function input(
  partial: Partial<RouteInput> & Pick<RouteInput, "viewChunks">,
): RouteInput {
  return {
    epoch: 1,
    current: { provider: "anthropic", model: "claude-sonnet-5" },
    candidates: both(),
    decision: decision(),
    budget: 40000,
    ...partial,
  };
}

describe("scenario 4 sensitive filter", () => {
  const filter = sensitivePolicy({
    rules: [{ pattern: ENV, denyProviders: ["openai"] }],
  });

  it("removes denied providers when a visible chunk touches .env", () => {
    const filtered = filter(
      input({
        viewChunks: [chunk("full", ".env"), chunk("full", "src/app.ts")],
      }),
    );
    expect(
      filtered.candidates.some((entry) => entry.provider === "openai"),
    ).toBe(false);
    expect(
      filtered.candidates.some((entry) => entry.provider === "anthropic"),
    ).toBe(true);
    expect(filtered.reasons).toContain("sensitive:.env");
  });

  it("ignores a matching chunk whose tier is none", () => {
    const filtered = filter(
      input({
        viewChunks: [chunk("none", ".env"), chunk("none", "proj/secrets/key")],
      }),
    );
    expect(
      filtered.candidates.some((entry) => entry.provider === "openai"),
    ).toBe(true);
    expect(filtered.reasons).toEqual([]);
  });

  it("matches secrets paths and line tier, and leaves unrelated keys alone", () => {
    const secrets = filter(
      input({ viewChunks: [chunk("line", "/work/secrets/token")] }),
    );
    expect(secrets.reasons).toEqual(["sensitive:/work/secrets/token"]);
    expect(
      secrets.candidates.every((entry) => entry.provider !== "openai"),
    ).toBe(true);
    const plain = filter(
      input({ viewChunks: [chunk("full", "src/.env.bak")] }),
    );
    expect(plain.reasons).toEqual([]);
    expect(plain.candidates.some((entry) => entry.provider === "openai")).toBe(
      true,
    );
  });
});

describe("scenario 5 cost tier", () => {
  const policy = costTierPolicy({
    ladder: [...LADDER],
    pick: "cheapest-allowed",
  });

  it("picks the lowest catalog input price on the ladder", () => {
    const result = policy(input({ viewChunks: [] }));
    expect(result?.model).toEqual({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    });
    expect(result?.reason).toContain("cheapest-allowed");
  });

  it("skips a cheaper ladder model the catalog cannot price", () => {
    const result = costTierPolicy({
      ladder: [
        { provider: "openai", model: "gpt-not-in-catalog" },
        { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
      ],
      pick: "cheapest-allowed",
    })(
      input({
        viewChunks: [],
        candidates: [
          ...both(),
          {
            provider: "openai",
            model: "gpt-not-in-catalog",
            contextWindow: 100,
            pricing: {
              inputPerMTok: 0.01,
              outputPerMTok: 0,
              cacheReadPerMTok: 0,
              cacheWritePerMTok: 0,
              source: "missing",
            },
          },
        ],
      }),
    );
    expect(result?.model).toEqual({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    });
  });

  it("ignores cheaper catalog models that are not on the ladder", () => {
    const result = costTierPolicy({
      ladder: [{ provider: "openai", model: "gpt-5.4" }],
      pick: "cheapest-allowed",
    })(input({ viewChunks: [] }));
    expect(result?.model).toEqual({ provider: "openai", model: "gpt-5.4" });
  });

  it("returns undefined when every ladder entry is missing a catalog price", () => {
    const result = costTierPolicy({
      ladder: [{ provider: "openai", model: "gpt-not-in-catalog" }],
      pick: "cheapest-allowed",
    })(
      input({
        viewChunks: [],
        candidates: [
          {
            provider: "openai",
            model: "gpt-not-in-catalog",
            contextWindow: 100,
            pricing: {
              inputPerMTok: 0.01,
              outputPerMTok: 0,
              cacheReadPerMTok: 0,
              cacheWritePerMTok: 0,
              source: "missing",
            },
          },
        ],
      }),
    );
    expect(result).toBeUndefined();
  });
});

describe("policy composition", () => {
  it("filters candidates before the cost policy and keeps sensitive reasons", () => {
    const policy = withSensitiveFilter(
      costTierPolicy({
        ladder: [{ provider: "openai", model: "gpt-5.4-mini" }, ...LADDER],
        pick: "cheapest-allowed",
      }),
      sensitivePolicy({
        rules: [{ pattern: ENV, denyProviders: ["openai"] }],
      }),
    );
    const open = policy(input({ viewChunks: [chunk("full", "README.md")] }));
    expect(open?.model).toEqual({ provider: "openai", model: "gpt-5.4-mini" });
    expect(open?.reason).not.toContain("sensitive:");

    const closed = policy(input({ viewChunks: [chunk("full", "app/.env")] }));
    expect(closed?.model.provider).not.toBe("openai");
    expect(closed?.model).toEqual({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    });
    expect(closed?.reason).toContain("sensitive:app/.env");

    const hidden = policy(input({ viewChunks: [chunk("none", ".env")] }));
    expect(hidden?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4-mini",
    });
    expect(hidden?.reason).not.toContain("sensitive:");
  });

  it("does not keep a model the filter removed", () => {
    const stubborn: RoutePolicy = () => ({
      model: { provider: "openai", model: "gpt-5.4" },
      reason: "stubborn",
    });
    const policy = withSensitiveFilter(
      stubborn,
      sensitivePolicy({
        rules: [{ pattern: ENV, denyProviders: ["openai"] }],
      }),
    );
    expect(
      policy(input({ viewChunks: [chunk("full", ".env")] })),
    ).toBeUndefined();
  });

  it("threads the previous model forward and keeps it when a later policy abstains", () => {
    const seen: { provider: string; model: string }[] = [];
    const first: RoutePolicy = () => ({
      model: { provider: "openai", model: "gpt-5.4-mini" },
      reason: "first",
    });
    const abstain: RoutePolicy = (routeInput) => {
      seen.push(routeInput.current);
      return undefined;
    };
    const second: RoutePolicy = (routeInput) => {
      seen.push(routeInput.current);
      return {
        model: { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
        reason: "second",
      };
    };
    const held = composePolicies(first, abstain)(input({ viewChunks: [] }));
    expect(held).toEqual({
      model: { provider: "openai", model: "gpt-5.4-mini" },
      reason: "first",
    });
    expect(seen[0]).toEqual({ provider: "openai", model: "gpt-5.4-mini" });

    const replaced = composePolicies(first, second)(input({ viewChunks: [] }));
    expect(replaced?.reason).toBe("second");
    expect(replaced?.model.model).toBe("claude-haiku-4-5-20251001");
    expect(seen[1]).toEqual({ provider: "openai", model: "gpt-5.4-mini" });

    const skipped = composePolicies(abstain)(input({ viewChunks: [] }));
    expect(skipped).toBeUndefined();
  });
});
