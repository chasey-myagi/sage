/**
 * One-shot coding CLI.
 *
 * Contract: parse argv, assemble AgentSession with the seven tools plus
 * update_ledger, expand, spawn_subtask, and spawn_subtasks, and a
 * JsonlLogStore under `<cwd>/.sage/sessions`. Subtask presets load from
 * `<cwd>/.sage/subtasks` and, on the CLI, from `homedir()`. The view
 * is contextCompiler with budget 40000, the four first-party claims, and
 * the default route policy, unless `--passthrough` selects passthroughView.
 * `--budget <tokens>` replaces 40000. The value must be a positive integer;
 * anything else exits 2 with one stderr line. `--passthrough` ignores a
 * valid `--budget` and writes one stderr line.
 * `--max-turns <n>` replaces the kernel default of 80. The value must be a
 * positive integer; anything else exits 2 with one stderr line. It is passed
 * to `session.run` under both the compiler and `--passthrough`.
 * `--tool-network none|full` defaults to full. `none` cuts network only for
 * bash child processes. On a platform that cannot enforce it, sage exits 2
 * before the run. Provider calls stay in this process.
 * `--scorer model` installs a model scorer on the root compiler. It reuses
 * the assembled adapters and the session model. `--scorer jev` installs the
 * TypeSafe Jev scorer. `JEV_BASE_URL` defaults to `https://api.typesafe.ai`.
 * `JEV_API_KEY` falls back to `TYPESAFE_API_KEY`. `JEV_MODEL` defaults to
 * `jev-latest`. When neither key is set, the process exits 2 and names both
 * variables. The default scorer is the deterministic rule scorer.
 * `--shadow-scorer rule|model|jev` installs a second scorer that records
 * `manifest.shadow` and does not change tiers. It is off unless the flag
 * is set. `jev` for either flag uses the same three variables; a missing
 * key still exits 2.
 * `--scorer-apply entry|rebuild` defaults to rebuild. `entry` scores a new
 * tool chunk when it first enters the view and requires `--scorer model`
 * or `--scorer jev`; otherwise the process exits 2. `entry` does not run
 * a shadow scorer. `--passthrough` does not compile, so neither scorer
 * is called.
 * `--provider typesafe`
 * is rejected: that catalog row prices Jev and is not a chat endpoint.
 * Each preset with its env key assembles an adapter. OpenAI-compatible
 * presets share one adapter class and differ by base URL. `--provider`
 * selects a preset. `--base-url` and `--api-key-env` override that preset.
 * A named preset with no key exits 2 and names the env var. With no
 * `--provider`, the first of deepseek, dashscope, moonshot, openai,
 * anthropic whose env key is set is used. If none of those keys are set,
 * the process exits 1. `--provider` still overrides that order.
 * The default route ladder is only the current model, so a rebuild does not
 * switch models. `--ladder` takes `provider/model` rungs separated by commas
 * and opts into cheapest-allowed across that list. `.env` and `secrets`
 * still drop every non-home provider on that ladder. stdout is one JSON
 * RunSummary, including `health`. `--json` asks for that same line.
 * `stats <session.jsonl>` reads the file with JsonlLogStore and prints
 * `runHealth` plus `shadowNone`, `laterNeeded`, and `precision`, without
 * starting a run. `shadowNone` is the number of distinct chunk ids a shadow
 * decision marked `none`. `laterNeeded` counts those ids that later appear
 * as an `expand` tool input `id`, or that are a `read` result whose `path`
 * is read again on a strictly later turn. `precision` is
 * `1 - laterNeeded / shadowNone`, or null when `shadowNone` is 0.
 * `bench run` clones each task, runs sage, then the task's test command,
 * and writes `<out>/results.json` plus `<out>/sessions/<taskId>.jsonl`.
 * `--budget` on `bench run` is forwarded on each `sage -p` argv.
 * `results.json` records that number, or null when the flag is omitted
 * or the policy is passthrough. `bench report` copies it onto `a` and `b`.
 * `--max-turns` on `bench run` is forwarded the same way. `results.json`
 * records that positive integer, or 80 when the flag is omitted.
 * It exits 2 when the selected provider has no API key. `bench report`
 * prints JSON `{ a, b, delta }` for two result directories. The kernel
 * appends each turn's manifest. Exit 0 when the outcome is done, otherwise
 * 1. `stats` and a completed bench command exit 0 after printing.
 *
 * Intentionally not in the Log: argv, API keys, the stderr progress
 * lines, and the stderr note that `--passthrough` ignored `--budget`.
 * `--tool-network` is argv. bash records `network: "none"` through the
 * kernel's toolResults, not as its own entry. The system prompt is
 * model-visible and is stored on request_header. Claim text reaches the
 * Log only through the manifest the kernel appends.
 */

import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  AgentSession,
  JsonlLogStore,
  type LogStore,
  passthroughView,
  runHealth,
  type StoredEntry,
} from "@sage/core";
import {
  agentsMd,
  contextCompiler,
  costTierPolicy,
  expandTool,
  jevScorer,
  modelScorer,
  progressLedger,
  type RoutePolicy,
  ruleScorer,
  type Scorer,
  sensitivePolicy,
  subtaskTools,
  systemReminder,
  toolCatalog,
  withSensitiveFilter,
} from "@sage/plugins";
import {
  AnthropicAdapter,
  defaultModel,
  listModels,
  type ModelRef,
  OpenAIAdapter,
  PRESETS,
  type ProviderAdapter,
  resolvePreset,
  type Sampling,
} from "@sage/provider";
import {
  type BashExec,
  createTools,
  type ToolNetwork,
  toolNetworkSupport,
} from "@sage/tools";
import { type BenchDeps, runBenchCli } from "./bench.js";

export type CodingSession = {
  session: AgentSession;
  store: LogStore;
  sessionId: string;
};

export function createCodingSession(opts: {
  cwd: string;
  adapter?: ProviderAdapter;
  adapters?: Record<string, ProviderAdapter>;
  model: ModelRef;
  system?: string;
  sessionId?: string;
  sessionsDir?: string;
  store?: LogStore;
  exec?: BashExec;
  network?: ToolNetwork;
  platform?: NodeJS.Platform;
  unshareAvailable?: boolean;
  sampling?: Sampling;
  budget?: number;
  passthrough?: boolean;
  scorer?: Scorer;
  shadowScorer?: Scorer;
  scorerApply?: "rebuild" | "entry";
  ladder?: readonly { provider: string; model: string }[];
  home?: string;
}): CodingSession {
  if (!opts.adapter && !opts.adapters) {
    throw new Error("createCodingSession requires adapter or adapters");
  }
  const adapters =
    opts.adapters ??
    ({
      [opts.model.provider]: opts.adapter,
    } as Record<string, ProviderAdapter>);
  if (!adapters[opts.model.provider]) {
    throw new Error(`No adapter for provider: ${opts.model.provider}`);
  }
  const sessionId = opts.sessionId ?? randomUUID();
  const store =
    opts.store ??
    new JsonlLogStore({
      dir: opts.sessionsDir ?? path.join(opts.cwd, ".sage", "sessions"),
    });
  const ledger = progressLedger();
  const system = opts.system ?? systemPrompt(opts.cwd);
  const claims = [
    systemReminder({ text: "Work in small verified steps." }),
    agentsMd({ cwd: opts.cwd }),
    toolCatalog({ deferred: [] }),
    ledger.provider,
  ];
  const route = defaultRoutePolicy(
    Object.keys(adapters),
    opts.model.provider,
    opts.ladder,
  );
  const tools = [
    ...createTools({
      cwd: opts.cwd,
      exec: opts.exec,
      network: opts.network,
      platform: opts.platform,
      ...(opts.unshareAvailable !== undefined
        ? { unshareAvailable: opts.unshareAvailable }
        : {}),
    }),
    ledger.tool,
    expandTool({ store, sessionId }),
  ];
  const { spawn, spawnMany } = subtaskTools({
    store,
    adapters,
    tools,
    system,
    cwd: opts.cwd,
    ...(opts.home ? { home: opts.home } : {}),
    makeCompiler: ({ budget, query }) =>
      contextCompiler({
        budget,
        claims,
        subtask: { query },
        expectedTurns: 1,
        route,
      }),
  });
  tools.push(spawn, spawnMany);
  const session = new AgentSession({
    store,
    sessionId,
    adapters,
    model: opts.model,
    system,
    tools,
    viewProvider: opts.passthrough
      ? passthroughView
      : contextCompiler({
          budget: opts.budget ?? 40000,
          claims,
          route,
          ...(opts.scorer ? { scorer: opts.scorer } : {}),
          ...(opts.shadowScorer ? { shadowScorer: opts.shadowScorer } : {}),
          ...(opts.scorerApply ? { scorerApply: opts.scorerApply } : {}),
        }),
    sampling: opts.sampling,
  });
  return { session, store, sessionId };
}

type JevFetchImpl = NonNullable<Parameters<typeof jevScorer>[0]["fetchImpl"]>;

export type CliIo = {
  env: NodeJS.ProcessEnv;
  cwd: string;
  stdout: { write(chunk: string): void };
  stderr: { write(chunk: string): void };
  adapter?: ProviderAdapter;
  adapters?: Record<string, ProviderAdapter>;
  /** Test seam. Production runs use git, the sage bin, and the test command. */
  benchDeps?: BenchDeps;
  exec?: BashExec;
  /** Test seam. Production uses process.platform. */
  platform?: NodeJS.Platform;
  /** Test seam. Production probes `unshare -n` on linux. */
  unshareAvailable?: boolean;
  /** Test seam. Production jev scoring uses global fetch. */
  jevFetch?: JevFetchImpl;
};

export async function runCli(
  argv: string[],
  io: Partial<CliIo> = {},
): Promise<number> {
  const stdout = io.stdout ?? process.stdout;
  const stderr = io.stderr ?? process.stderr;
  const cwd = io.cwd ?? process.cwd();
  const env = io.env ?? process.env;
  if (argv[0] === "stats") {
    return runStats(argv.slice(1), { cwd, stdout, stderr });
  }
  if (argv[0] === "bench") {
    return runBenchCli(argv.slice(1), {
      env,
      cwd,
      stdout,
      stderr,
      ...(io.benchDeps ? { deps: io.benchDeps } : {}),
    });
  }
  const parsed = parseArgs(argv);
  if (!parsed.ok) {
    stderr.write(`${parsed.message}\n`);
    return parsed.exitCode ?? 1;
  }
  if (
    parsed.scorerApply === "entry" &&
    parsed.scorer !== "model" &&
    parsed.scorer !== "jev"
  ) {
    stderr.write("entry scoring needs --scorer model|jev\n");
    return 2;
  }
  const network = parsed.toolNetwork ?? "full";
  const platform = io.platform ?? process.platform;
  let unshareAvailable = io.unshareAvailable;
  if (network === "none") {
    const support = toolNetworkSupport({
      platform,
      ...(unshareAvailable !== undefined ? { unshareAvailable } : {}),
    });
    if (!support.ok) {
      stderr.write(`${support.message}\n`);
      return 2;
    }
    if (platform === "linux") unshareAvailable = true;
  }
  if (
    (parsed.scorer === "jev" || parsed.shadowScorer === "jev") &&
    !jevApiKey(env)
  ) {
    stderr.write("Missing JEV_API_KEY or TYPESAFE_API_KEY\n");
    return 2;
  }
  const auth = resolveAuth(
    parsed.provider,
    parsed.model,
    env,
    parsed.baseUrl,
    parsed.apiKeyEnv,
  );
  if (!auth.ok) {
    stderr.write(`${auth.message}\n`);
    return auth.exitCode;
  }
  const injected = Boolean(io.adapters || io.adapter);
  const adapters =
    io.adapters ??
    (io.adapter ? { [auth.provider]: io.adapter } : adaptersFromEnv(env));
  if (!injected || !adapters[auth.provider]) {
    adapters[auth.provider] = createAdapter(
      auth.provider,
      auth.apiKey,
      auth.baseUrl,
    );
  }
  if (parsed.passthrough && parsed.budget !== undefined) {
    stderr.write(
      "Ignoring --budget because --passthrough does not compile a context\n",
    );
  }
  const { session } = createCodingSession({
    cwd,
    adapters,
    model: { provider: auth.provider, model: auth.model },
    passthrough: parsed.passthrough,
    home: homedir(),
    network,
    platform,
    ...(unshareAvailable !== undefined ? { unshareAvailable } : {}),
    ...(io.exec ? { exec: io.exec } : {}),
    ...(parsed.budget !== undefined && !parsed.passthrough
      ? { budget: parsed.budget }
      : {}),
    ...(parsed.ladder ? { ladder: parsed.ladder } : {}),
    scorer: namedScorer(
      parsed.scorer,
      adapters,
      { provider: auth.provider, model: auth.model },
      env,
      io.jevFetch,
    ),
    shadowScorer: namedScorer(
      parsed.shadowScorer,
      adapters,
      { provider: auth.provider, model: auth.model },
      env,
      io.jevFetch,
    ),
    ...(parsed.scorerApply ? { scorerApply: parsed.scorerApply } : {}),
  });
  let turn = 0;
  session.hooks.observe("turn_start", (payload) => {
    turn = payload.turn;
  });
  session.hooks.observe("llm_end", ({ assistant }) => {
    const usage = assistant.usage;
    const input =
      usage.inputOther + usage.inputCacheRead + usage.inputCacheCreation;
    const calls = assistant.content.filter(
      (block) => block.type === "tool_use",
    ).length;
    stderr.write(
      `turn ${turn} · tokens ${input}/${usage.output} · tool calls ${calls}\n`,
    );
  });
  try {
    const summary = await session.run(
      parsed.task,
      parsed.maxTurns !== undefined ? { maxTurns: parsed.maxTurns } : undefined,
    );
    stdout.write(`${JSON.stringify(summary)}\n`);
    return summary.outcome === "done" ? 0 : 1;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    stderr.write(`${message}\n`);
    return 1;
  }
}

const SENSITIVE = /(^|\/)\.env$|\/secrets\//;

export function defaultRoutePolicy(
  providers: readonly string[],
  home: string,
  ladder?: readonly { provider: string; model: string }[],
): RoutePolicy {
  const unique = [...new Set(providers)];
  const candidates = unique.flatMap((provider) => [...listModels(provider)]);
  const deny = unique.filter((provider) => provider !== home);
  const filter = sensitivePolicy({
    rules:
      deny.length === 0 ? [] : [{ pattern: SENSITIVE, denyProviders: deny }],
  });
  const explicit =
    ladder && ladder.length > 0
      ? ladder.map((entry) => ({
          provider: entry.provider,
          model: entry.model,
        }))
      : undefined;
  return (input) => {
    const policy = withSensitiveFilter(
      costTierPolicy({
        ladder: explicit ?? [input.current],
        pick: "cheapest-allowed",
      }),
      filter,
    );
    return policy({ ...input, candidates });
  };
}

export function adaptersFromEnv(
  env: NodeJS.ProcessEnv,
): Record<string, ProviderAdapter> {
  const adapters: Record<string, ProviderAdapter> = {};
  for (const preset of PRESETS) {
    const apiKey = env[preset.envKey];
    if (!apiKey) continue;
    adapters[preset.name] = createAdapter(preset.name, apiKey, preset.baseUrl);
  }
  return adapters;
}

function runStats(
  argv: string[],
  io: {
    cwd: string;
    stdout: { write(chunk: string): void };
    stderr: { write(chunk: string): void };
  },
): number {
  const file = argv[0];
  if (argv.length !== 1 || !file || file.startsWith("-")) {
    io.stderr.write(`${usage()}\n`);
    return 1;
  }
  const resolved = path.resolve(io.cwd, file);
  const sessionId = path.basename(resolved, ".jsonl");
  if (!resolved.endsWith(".jsonl") || sessionId.length === 0) {
    io.stderr.write(`${usage()}\n`);
    return 1;
  }
  if (!existsSync(resolved)) {
    io.stderr.write(`Missing session file: ${resolved}\n`);
    return 1;
  }
  try {
    const store = new JsonlLogStore({ dir: path.dirname(resolved) });
    const logged = store.path(sessionId);
    const health = runHealth(logged);
    const audit = shadowAudit(logged);
    io.stdout.write(`${JSON.stringify({ ...health, ...audit })}\n`);
    return 0;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    io.stderr.write(`${message}\n`);
    return 1;
  }
}

function systemPrompt(cwd: string): string {
  return [
    "You are sage, a coding agent.",
    "Complete the task in the workspace using the provided tools.",
    "Read before editing. Use edit for a single exact replacement.",
    "Stop when the task is done.",
    `Workspace: ${cwd}`,
  ].join("\n");
}

type ScorerName = "rule" | "model" | "jev";

type FlagName =
  | "print"
  | "provider"
  | "model"
  | "scorer"
  | "shadow-scorer"
  | "scorer-apply"
  | "ladder"
  | "base-url"
  | "api-key-env"
  | "budget"
  | "max-turns"
  | "tool-network";

type Parsed =
  | {
      ok: true;
      task: string;
      provider?: string;
      model?: string;
      baseUrl?: string;
      apiKeyEnv?: string;
      passthrough: boolean;
      json: boolean;
      scorer?: ScorerName;
      shadowScorer?: ScorerName;
      scorerApply?: "rebuild" | "entry";
      ladder?: { provider: string; model: string }[];
      budget?: number;
      maxTurns?: number;
      toolNetwork?: ToolNetwork;
    }
  | { ok: false; message: string; exitCode?: number };

export function parseArgs(argv: string[]): Parsed {
  let task: string | undefined;
  let provider: string | undefined;
  let model: string | undefined;
  let baseUrl: string | undefined;
  let apiKeyEnv: string | undefined;
  let passthrough = false;
  let json = false;
  let scorer: ScorerName | undefined;
  let shadowScorer: ScorerName | undefined;
  let scorerApply: "rebuild" | "entry" | undefined;
  let ladder: { provider: string; model: string }[] | undefined;
  let budget: number | undefined;
  let maxTurns: number | undefined;
  let toolNetwork: ToolNetwork | undefined;
  const positional: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index] ?? "";
    if (arg === "--help" || arg === "-h") {
      return { ok: false, message: usage() };
    }
    if (arg === "--passthrough") {
      passthrough = true;
      continue;
    }
    if (arg === "--json") {
      json = true;
      continue;
    }
    if (arg === "--print" || arg === "-p") {
      const next = argv[index + 1];
      if (next && !next.startsWith("-")) {
        task = next;
        index += 1;
      }
      continue;
    }
    const taken = take(argv, index, arg);
    if (taken.kind === "unknown") return { ok: false, message: taken.message };
    if (taken.kind === "flag") {
      index = taken.next;
      if (taken.name === "print") task = taken.value;
      if (taken.name === "provider") provider = taken.value;
      if (taken.name === "model") model = taken.value;
      if (taken.name === "scorer") {
        if (!isScorerName(taken.value)) {
          return { ok: false, message: usage() };
        }
        scorer = taken.value;
      }
      if (taken.name === "shadow-scorer") {
        if (!isScorerName(taken.value)) {
          return { ok: false, message: usage() };
        }
        shadowScorer = taken.value;
      }
      if (taken.name === "scorer-apply") {
        if (taken.value !== "entry" && taken.value !== "rebuild") {
          return { ok: false, message: usage() };
        }
        scorerApply = taken.value;
      }
      if (taken.name === "ladder") {
        const parsed = parseLadder(taken.value);
        if (!parsed) return { ok: false, message: usage() };
        ladder = parsed;
      }
      if (taken.name === "base-url") baseUrl = taken.value;
      if (taken.name === "api-key-env") apiKeyEnv = taken.value;
      if (taken.name === "budget") {
        const parsedBudget = positiveInt(taken.value);
        if (parsedBudget === undefined) {
          return {
            ok: false,
            exitCode: 2,
            message: invalidBudget(taken.value),
          };
        }
        budget = parsedBudget;
      }
      if (taken.name === "max-turns") {
        const parsedMaxTurns = positiveInt(taken.value);
        if (parsedMaxTurns === undefined) {
          return {
            ok: false,
            exitCode: 2,
            message: invalidMaxTurns(taken.value),
          };
        }
        maxTurns = parsedMaxTurns;
      }
      if (taken.name === "tool-network") {
        if (taken.value !== "none" && taken.value !== "full") {
          return {
            ok: false,
            exitCode: 2,
            message: invalidToolNetwork(taken.value),
          };
        }
        toolNetwork = taken.value;
      }
      continue;
    }
    positional.push(arg);
  }
  if (task !== undefined && positional.length > 0) {
    return { ok: false, message: usage() };
  }
  if (task === undefined) task = positional.join(" ");
  if (!task) return { ok: false, message: usage() };
  return {
    ok: true,
    task,
    provider,
    model,
    ...(baseUrl ? { baseUrl } : {}),
    ...(apiKeyEnv ? { apiKeyEnv } : {}),
    passthrough,
    json,
    scorer,
    shadowScorer,
    ...(scorerApply ? { scorerApply } : {}),
    ladder,
    ...(budget !== undefined ? { budget } : {}),
    ...(maxTurns !== undefined ? { maxTurns } : {}),
    ...(toolNetwork !== undefined ? { toolNetwork } : {}),
  };
}

function usage(): string {
  return [
    "usage: sage [-p | --print] <task> [--provider <preset>] [--model <id>] [--base-url <url>] [--api-key-env <NAME>] [--ladder provider/model,provider/model] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--passthrough] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild] [--json]",
    "       sage stats <session.jsonl>",
    "       sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild] --out <dir>",
    "       sage bench report <runA> <runB>",
  ].join("\n");
}

function take(
  argv: string[],
  index: number,
  arg: string,
):
  | {
      kind: "flag";
      name: FlagName;
      value: string;
      next: number;
    }
  | { kind: "positional" }
  | { kind: "unknown"; message: string } {
  const inline = inlineFlag(arg);
  if (inline) return { kind: "flag", ...inline, next: index };
  const name = flagName(arg);
  if (name === "unknown") return { kind: "unknown", message: usage() };
  if (!name) return { kind: "positional" };
  const value = argv[index + 1];
  if (!value || value.startsWith("-")) {
    return { kind: "unknown", message: usage() };
  }
  return { kind: "flag", name, value, next: index + 1 };
}

function flagName(arg: string): FlagName | "unknown" | undefined {
  if (arg === "--print" || arg === "-p") return "print";
  if (arg === "--provider") return "provider";
  if (arg === "--model") return "model";
  if (arg === "--scorer") return "scorer";
  if (arg === "--shadow-scorer") return "shadow-scorer";
  if (arg === "--scorer-apply") return "scorer-apply";
  if (arg === "--ladder") return "ladder";
  if (arg === "--base-url") return "base-url";
  if (arg === "--api-key-env") return "api-key-env";
  if (arg === "--budget") return "budget";
  if (arg === "--max-turns") return "max-turns";
  if (arg === "--tool-network") return "tool-network";
  if (arg.startsWith("-")) return "unknown";
  return undefined;
}

function inlineFlag(
  arg: string,
): { name: FlagName; value: string } | undefined {
  const names = [
    "print",
    "provider",
    "model",
    "scorer",
    "shadow-scorer",
    "scorer-apply",
    "ladder",
    "base-url",
    "api-key-env",
    "budget",
    "max-turns",
    "tool-network",
  ] as const;
  for (const name of names) {
    const prefix = `--${name}=`;
    if (!arg.startsWith(prefix)) continue;
    const value = arg.slice(prefix.length);
    if (!value) return undefined;
    return { name, value };
  }
  return undefined;
}

function positiveInt(value: string): number | undefined {
  if (!/^[0-9]+$/.test(value)) return undefined;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) return undefined;
  return parsed;
}

function invalidBudget(value: string): string {
  return `Invalid --budget ${JSON.stringify(value)}: expected a positive integer`;
}

function invalidMaxTurns(value: string): string {
  return `Invalid --max-turns ${JSON.stringify(value)}: expected a positive integer`;
}

function invalidToolNetwork(value: string): string {
  return `Invalid --tool-network ${JSON.stringify(value)}: expected none or full`;
}

function parseLadder(
  value: string,
): { provider: string; model: string }[] | undefined {
  const parts = value
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  if (parts.length === 0) return undefined;
  const ladder: { provider: string; model: string }[] = [];
  for (const part of parts) {
    const slash = part.indexOf("/");
    if (slash <= 0 || slash === part.length - 1) return undefined;
    ladder.push({
      provider: part.slice(0, slash),
      model: part.slice(slash + 1),
    });
  }
  return ladder;
}

const AUTO_PROVIDERS = [
  "deepseek",
  "dashscope",
  "moonshot",
  "openai",
  "anthropic",
] as const;

function autoProvider(env: NodeJS.ProcessEnv): string | undefined {
  for (const name of AUTO_PROVIDERS) {
    const preset = resolvePreset(name);
    if (preset && env[preset.envKey]) return name;
  }
  return undefined;
}

function resolveAuth(
  provider: string | undefined,
  model: string | undefined,
  env: NodeJS.ProcessEnv,
  baseUrl: string | undefined,
  apiKeyEnv: string | undefined,
):
  | {
      ok: true;
      provider: string;
      model: string;
      apiKey: string;
      baseUrl: string;
    }
  | { ok: false; message: string; exitCode: number } {
  let selected = provider;
  if (!selected) {
    selected = autoProvider(env);
    if (!selected) {
      return {
        ok: false,
        exitCode: 1,
        message:
          "Missing DEEPSEEK_API_KEY, DASHSCOPE_API_KEY, MOONSHOT_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY",
      };
    }
  }
  if (selected === "typesafe") {
    return {
      ok: false,
      exitCode: 1,
      message:
        "typesafe is not a chat endpoint; use --scorer jev with JEV_API_KEY or TYPESAFE_API_KEY",
    };
  }
  const preset = resolvePreset(selected);
  if (!preset) {
    return { ok: false, exitCode: 1, message: `Unknown provider: ${selected}` };
  }
  const envKey = apiKeyEnv ?? preset.envKey;
  const apiKey = env[envKey];
  if (!apiKey) {
    return { ok: false, exitCode: 2, message: `Missing ${envKey}` };
  }
  return {
    ok: true,
    provider: selected,
    model: model ?? defaultModel(selected).model,
    apiKey,
    baseUrl: baseUrl ?? preset.baseUrl,
  };
}

function createAdapter(
  provider: string,
  apiKey: string,
  baseUrl: string,
): ProviderAdapter {
  const preset = resolvePreset(provider);
  if (preset?.api === "anthropic-messages") {
    return new AnthropicAdapter({ apiKey, baseUrl });
  }
  return new OpenAIAdapter({ apiKey, baseUrl });
}

function isScorerName(value: string): value is ScorerName {
  return value === "rule" || value === "model" || value === "jev";
}

function jevApiKey(env: NodeJS.ProcessEnv): string | undefined {
  return env.JEV_API_KEY || env.TYPESAFE_API_KEY || undefined;
}

function namedScorer(
  name: ScorerName | undefined,
  adapters: Record<string, ProviderAdapter>,
  model: ModelRef,
  env: NodeJS.ProcessEnv,
  jevFetch?: JevFetchImpl,
): Scorer | undefined {
  if (name === "model") return modelScorer({ adapters, model });
  if (name === "jev") {
    const baseUrl = env.JEV_BASE_URL;
    const jevModel = env.JEV_MODEL;
    return jevScorer({
      apiKey: jevApiKey(env),
      ...(baseUrl ? { baseUrl } : {}),
      ...(jevModel ? { model: jevModel } : {}),
      ...(jevFetch ? { fetchImpl: jevFetch } : {}),
    });
  }
  if (name === "rule") return ruleScorer;
  return undefined;
}

function shadowAudit(entries: readonly StoredEntry[]): {
  shadowNone: number;
  laterNeeded: number;
  precision: number | null;
} {
  const judgedTurn = new Map<string, number>();
  const judgedAt = new Map<string, number>();
  const calls: {
    index: number;
    turn: number;
    name: string;
    toolUseId: string;
    input: unknown;
  }[] = [];
  let currentTurn = 0;
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index];
    if (!entry) continue;
    if (entry.kind === "manifest" && isRecord(entry.payload)) {
      if (isFiniteNumber(entry.payload.turn)) currentTurn = entry.payload.turn;
      rememberShadow(
        entry.payload.shadow,
        index,
        currentTurn,
        judgedTurn,
        judgedAt,
      );
      continue;
    }
    if (entry.kind !== "message" || !isAssistantMessage(entry.payload))
      continue;
    for (const block of entry.payload.content) {
      if (!isRecord(block) || block.type !== "tool_use") continue;
      if (typeof block.name !== "string" || typeof block.id !== "string") {
        continue;
      }
      calls.push({
        index,
        turn: currentTurn,
        name: block.name,
        toolUseId: block.id,
        input: block.input,
      });
    }
  }
  let laterNeeded = 0;
  for (const [id, turn] of judgedTurn) {
    const at = judgedAt.get(id) ?? -1;
    if (expandedLater(id, at, calls) || rereadLater(id, turn, calls)) {
      laterNeeded += 1;
    }
  }
  const shadowNone = judgedTurn.size;
  return {
    shadowNone,
    laterNeeded,
    precision: shadowNone === 0 ? null : 1 - laterNeeded / shadowNone,
  };
}

function rememberShadow(
  shadow: unknown,
  index: number,
  turn: number,
  judgedTurn: Map<string, number>,
  judgedAt: Map<string, number>,
): void {
  if (!isRecord(shadow) || !Array.isArray(shadow.decisions)) return;
  for (const decision of shadow.decisions) {
    if (!isRecord(decision) || decision.tier !== "none") continue;
    if (typeof decision.id !== "string" || decision.id.length === 0) continue;
    if (judgedTurn.has(decision.id)) continue;
    judgedTurn.set(decision.id, turn);
    judgedAt.set(decision.id, index);
  }
}

function expandedLater(
  chunkId: string,
  at: number,
  calls: readonly { index: number; name: string; input: unknown }[],
): boolean {
  return calls.some(
    (call) =>
      call.index > at &&
      call.name === "expand" &&
      expandId(call.input) === chunkId,
  );
}

function rereadLater(
  chunkId: string,
  judgedTurn: number,
  calls: readonly {
    turn: number;
    name: string;
    toolUseId: string;
    input: unknown;
  }[],
): boolean {
  const toolUseId = toolUseIdOf(chunkId);
  if (!toolUseId) return false;
  const origin = calls.find((call) => call.toolUseId === toolUseId);
  if (origin?.name !== "read") return false;
  const file = readPath(origin.input);
  if (!file) return false;
  return calls.some(
    (call) =>
      call.name === "read" &&
      call.turn > judgedTurn &&
      call.toolUseId !== toolUseId &&
      readPath(call.input) === file,
  );
}

function toolUseIdOf(chunkId: string): string | undefined {
  const hash = chunkId.indexOf("#");
  if (hash <= 0 || hash === chunkId.length - 1) return undefined;
  return chunkId.slice(hash + 1);
}

function expandId(input: unknown): string | undefined {
  if (!isRecord(input)) return undefined;
  if (typeof input.id !== "string" || input.id.length === 0) return undefined;
  return input.id;
}

function readPath(input: unknown): string | undefined {
  if (!isRecord(input)) return undefined;
  if (typeof input.path !== "string" || input.path.length === 0) {
    return undefined;
  }
  return input.path;
}

function isAssistantMessage(
  value: unknown,
): value is { role: string; content: unknown[] } {
  return (
    isRecord(value) &&
    value.role === "assistant" &&
    Array.isArray(value.content)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isDirectRun(): boolean {
  const entry = process.argv[1];
  if (!entry) return false;
  return import.meta.url === pathToFileURL(path.resolve(entry)).href;
}

if (isDirectRun()) {
  runCli(process.argv.slice(2))
    .then((code) => {
      process.exit(code);
    })
    .catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      process.stderr.write(`${message}\n`);
      process.exit(1);
    });
}
