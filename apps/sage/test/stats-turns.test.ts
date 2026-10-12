import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  AgentSession,
  type ChunkRecord,
  JsonlLogStore,
  type Manifest,
  passthroughView,
  type ViewProvider,
} from "@sage/core";
import {
  createFaux,
  FAUX_MODEL,
  FAUX_SYSTEM,
  pathlessTool,
  stopScript,
  toolScript,
} from "@sage/core/testing";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

const TASK = "TASK_TEXT_SENTINEL do not print me";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

type Plan = {
  epoch: number;
  epochStart?: { reason: string };
  manifest: Pick<Manifest, "chunks" | "budget" | "epochDecision">;
};

function chunk(id: string, tier: ChunkRecord["tier"]): ChunkRecord {
  return { id, tier, reason: `reason-for-${id}` };
}

function decision(
  rebuild: boolean,
  oldTokens: number,
  newTokens: number,
  remainingTurns: number,
  reason: string,
): NonNullable<Manifest["epochDecision"]> {
  return {
    rebuild,
    oldTokens,
    newTokens,
    remainingTurns,
    readRate: 0.1,
    writeRate: 1.25,
    savingsPerTurn: 1,
    extraCost: 2,
    paybackTurns: rebuild ? 1 : null,
    reason,
  };
}

const PLANS: Plan[] = [
  {
    epoch: 0,
    manifest: {
      chunks: [chunk("seq:0", "full")],
      budget: { total: 100, used: 10, overrun: 0, omitted: 0, expands: 0 },
      epochDecision: decision(false, 10, 10, 6, "savings-too-small"),
    },
  },
  {
    epoch: 0,
    manifest: {
      chunks: [chunk("seq:0", "full"), chunk("t1", "full")],
      budget: { total: 100, used: 20, overrun: 0, omitted: 0, expands: 0 },
    },
  },
  {
    epoch: 0,
    manifest: {
      chunks: [
        chunk("seq:0", "summary"),
        chunk("t1", "none"),
        chunk("t2", "line"),
      ],
      budget: { total: 100, used: 15, overrun: 0, omitted: 1, expands: 0 },
    },
  },
  {
    epoch: 1,
    epochStart: { reason: "rebuild" },
    manifest: {
      chunks: [
        chunk("seq:0", "none"),
        chunk("t1", "none"),
        chunk("t2", "none"),
      ],
      budget: { total: 100, used: 5, overrun: 0, omitted: 3, expands: 1 },
      epochDecision: decision(
        true,
        15,
        5,
        3,
        "payback:1<=3;route:sensitive:/work/.env",
      ),
    },
  },
];

/** Passthrough messages with the turn's planned manifest and epoch. */
const plannedView: ViewProvider = {
  async view(ctx) {
    const base = await passthroughView.view(ctx);
    const plan = PLANS[ctx.turn - 1];
    if (!plan) throw new Error(`no plan for turn ${ctx.turn}`);
    return {
      ...base,
      epoch: plan.epoch,
      ...(plan.epochStart ? { epochStart: plan.epochStart } : {}),
      manifest: { ...base.manifest, epoch: plan.epoch, ...plan.manifest },
    };
  },
};

async function recordSession(): Promise<string> {
  dir = mkdtempSync(path.join(tmpdir(), "sage-stats-turns-"));
  const store = new JsonlLogStore({ dir });
  const { adapter } = createFaux([
    toolScript("t1", "probe", "{}", {
      inputOther: 10,
      inputCacheRead: 0,
      inputCacheCreation: 5,
      output: 2,
    }),
    toolScript("t2", "missing", "{}", {
      inputOther: 4,
      inputCacheRead: 16,
      inputCacheCreation: 0,
      output: 3,
    }),
    toolScript("t3", "probe", "{}", {
      inputOther: 2,
      inputCacheRead: 18,
      inputCacheCreation: 0,
      output: 1,
    }),
    stopScript("finished", {
      inputOther: 0,
      inputCacheRead: 0,
      inputCacheCreation: 0,
      output: 1,
    }),
  ]);
  const session = new AgentSession({
    store,
    sessionId: "turns",
    adapter,
    model: FAUX_MODEL,
    system: FAUX_SYSTEM,
    tools: [pathlessTool("probe")],
    viewProvider: plannedView,
  });
  const summary = await session.run(TASK);
  expect(summary.outcome).toBe("done");
  // A tool_use with no result and a result with no tool_use, as a crash or
  // a hand-edited file would leave them.
  store.append("turns", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "tool_use", id: "dangle", name: "probe", input: {} }],
    },
  });
  store.append("turns", {
    kind: "message",
    payload: {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "ghost",
          content: [{ type: "text", text: "orphan" }],
          isError: false,
        },
      ],
    },
  });
  return path.join(dir, "turns.jsonl");
}

function capture() {
  let stdout = "";
  let stderr = "";
  return {
    io: {
      stdout: {
        write(chunk: string) {
          stdout += chunk;
        },
      },
      stderr: {
        write(chunk: string) {
          stderr += chunk;
        },
      },
    },
    text: () => ({ stdout, stderr }),
  };
}

function tokens(
  input: number,
  cacheRead: number,
  cacheCreation: number,
  output: number,
) {
  return {
    input,
    cacheRead,
    cacheCreation,
    output,
    total: input + cacheRead + cacheCreation + output,
  };
}

const NO_TIERS = { full: 0, summary: 0, line: 0, none: 0 };

it("prints one row per turn with usage, budget, tier counts, flips, epoch decision, and tools", async () => {
  const file = await recordSession();
  const out = capture();
  const code = await runCli(["stats", "--turns", file], {
    cwd: dir,
    env: {},
    ...out.io,
  });
  expect(out.text().stderr).toBe("");
  expect(code).toBe(0);
  const lines = out.text().stdout.trimEnd().split("\n");
  expect(lines).toHaveLength(1);
  const printed = JSON.parse(lines[0] ?? "") as { turns: unknown };
  const model = { provider: FAUX_MODEL.provider, model: FAUX_MODEL.model };
  expect(printed.turns).toEqual([
    {
      turn: 1,
      epoch: 0,
      model,
      tokens: tokens(10, 0, 5, 2),
      cacheHitRatio: 0,
      budget: { used: 10, omitted: 0, expands: 0 },
      tiers: { ...NO_TIERS, full: 1 },
      flips: [],
      epochDecision: {
        rebuild: false,
        oldTokens: 10,
        newTokens: 10,
        remainingTurns: 6,
        paybackTurns: null,
      },
      tools: ["probe"],
    },
    {
      turn: 2,
      epoch: 0,
      model,
      tokens: tokens(4, 16, 0, 3),
      cacheHitRatio: 0.8,
      budget: { used: 20, omitted: 0, expands: 0 },
      tiers: { ...NO_TIERS, full: 2 },
      flips: [],
      epochDecision: null,
      tools: ["missing"],
    },
    {
      turn: 3,
      epoch: 0,
      model,
      tokens: tokens(2, 18, 0, 1),
      cacheHitRatio: 0.9,
      budget: { used: 15, omitted: 1, expands: 0 },
      tiers: { full: 0, summary: 1, line: 1, none: 1 },
      flips: [
        { id: "seq:0", from: "full", to: "summary" },
        { id: "t1", from: "full", to: "none" },
      ],
      epochDecision: null,
      tools: ["probe"],
    },
    {
      turn: 4,
      epoch: 1,
      model,
      tokens: tokens(0, 0, 0, 1),
      cacheHitRatio: null,
      budget: { used: 5, omitted: 3, expands: 1 },
      tiers: { ...NO_TIERS, none: 3 },
      flips: [],
      epochDecision: {
        rebuild: true,
        oldTokens: 15,
        newTokens: 5,
        remainingTurns: 3,
        paybackTurns: 1,
      },
      tools: [],
    },
  ]);
});

it("summarizes pairing, errors, entry counts, models, outcome, and health after the turns", async () => {
  const file = await recordSession();
  const out = capture();
  const code = await runCli(["stats", file, "--turns"], {
    cwd: dir,
    env: {},
    ...out.io,
  });
  expect(code).toBe(0);
  const printed = JSON.parse(out.text().stdout) as {
    invariants: Record<string, unknown> & {
      health: { turns: number; toolErrors: number; epochs: number };
    };
  };
  const { health, ...rest } = printed.invariants;
  expect(rest).toEqual({
    toolUses: 4,
    toolResults: 4,
    unpairedToolUses: ["dangle"],
    orphanToolResults: ["ghost"],
    toolErrors: 1,
    requestHeaders: 4,
    manifests: 4,
    models: [{ provider: FAUX_MODEL.provider, model: FAUX_MODEL.model }],
    outcome: "done",
  });
  expect(health).toMatchObject({ turns: 5, toolErrors: 1, epochs: 2 });
});

it("keeps message text, chunk reasons, decision reasons, and paths out of the turn report", async () => {
  const file = await recordSession();
  const out = capture();
  const code = await runCli(["stats", "--turns", file], {
    cwd: dir,
    env: {},
    ...out.io,
  });
  expect(code).toBe(0);
  const stdout = out.text().stdout;
  expect(stdout).toContain('"turns"');
  expect(stdout).not.toContain("TASK_TEXT_SENTINEL");
  expect(stdout).not.toContain("finished");
  expect(stdout).not.toContain("reason-for-");
  expect(stdout).not.toContain("/work/.env");
  expect(stdout).not.toContain("Unknown tool");
  expect(stdout).not.toContain(dir);
});

it("rejects --turns without a session file and a second file with the usage", async () => {
  const file = await recordSession();
  for (const argv of [
    ["stats", "--turns"],
    ["stats", "--turns", file, file],
    ["stats", "--turns=yes", file],
  ]) {
    const out = capture();
    const code = await runCli(argv, { cwd: dir, env: {}, ...out.io });
    expect(code).toBe(1);
    expect(out.text().stdout).toBe("");
    expect(out.text().stderr).toContain("sage stats [--turns] <session.jsonl>");
  }
});
