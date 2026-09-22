/**
 * One-shot coding CLI.
 *
 * Contract: parse argv, assemble AgentSession with the seven tools plus
 * update_ledger, expand, spawn_subtask, and spawn_subtasks, and a
 * JsonlLogStore under `<cwd>/.sage/sessions`. Subtask presets load from
 * `<cwd>/.sage/subtasks` and, on the CLI, from `homedir()`. The view
 * is contextCompiler with budget 40000, the four first-party claims, and
 * the default route policy, unless `--passthrough` selects passthroughView.
 * `--scorer model` installs a model scorer on the root compiler. It reuses
 * the assembled adapters and the session model. `--scorer jev` installs the
 * TypeSafe Jev scorer and reads `TYPESAFE_API_KEY`; a missing key exits 2.
 * The default scorer is the deterministic rule scorer. `--provider typesafe`
 * is rejected: that catalog row prices Jev and is not a chat endpoint.
 * Each preset with its env key assembles an adapter. OpenAI-compatible
 * presets share one adapter class and differ by base URL. `--provider`
 * selects a preset. `--base-url` and `--api-key-env` override that preset.
 * A named preset with no key exits 2 and names the env var. With no
 * `--provider` and neither Anthropic nor OpenAI key, the process exits 1.
 * The default route ladder is only the current model, so a rebuild does not
 * switch models. `--ladder` takes `provider/model` rungs separated by commas
 * and opts into cheapest-allowed across that list. `.env` and `secrets`
 * still drop every non-home provider on that ladder. stdout is one JSON
 * RunSummary, including `health`. `--json` asks for that same line.
 * `stats <session.jsonl>` reads the file with JsonlLogStore and prints
 * `runHealth` without starting a run.
 * `bench run` clones each task, runs sage, then the task's test command,
 * and writes `<out>/results.json` plus `<out>/sessions/<taskId>.jsonl`.
 * It exits 2 when the selected provider has no API key. `bench report`
 * prints JSON `{ a, b, delta }` for two result directories. The kernel
 * appends each turn's manifest. Exit 0 when the outcome is done, otherwise
 * 1. `stats` and a completed bench command exit 0 after printing.
 *
 * Intentionally not in the Log: argv, API keys, and the stderr progress
 * lines. The system prompt is model-visible and is stored on request_header.
 * Claim text reaches the Log only through the manifest the kernel appends.
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
import { type BashExec, createTools } from "@sage/tools";
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
  sampling?: Sampling;
  budget?: number;
  passthrough?: boolean;
  scorer?: Scorer;
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
    ...createTools({ cwd: opts.cwd, exec: opts.exec }),
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
        }),
    sampling: opts.sampling,
  });
  return { session, store, sessionId };
}

export type CliIo = {
  env: NodeJS.ProcessEnv;
  cwd: string;
  stdout: { write(chunk: string): void };
  stderr: { write(chunk: string): void };
  adapter?: ProviderAdapter;
  adapters?: Record<string, ProviderAdapter>;
  /** Test seam. Production runs use git, the sage bin, and the test command. */
  benchDeps?: BenchDeps;
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
    return 1;
  }
  if (parsed.scorer === "jev" && !env.TYPESAFE_API_KEY) {
    stderr.write("Missing TYPESAFE_API_KEY\n");
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
  const { session } = createCodingSession({
    cwd,
    adapters,
    model: { provider: auth.provider, model: auth.model },
    passthrough: parsed.passthrough,
    home: homedir(),
    ...(parsed.ladder ? { ladder: parsed.ladder } : {}),
    scorer:
      parsed.scorer === "model"
        ? modelScorer({
            adapters,
            model: { provider: auth.provider, model: auth.model },
          })
        : parsed.scorer === "jev"
          ? jevScorer({ apiKey: env.TYPESAFE_API_KEY })
          : undefined,
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
    const summary = await session.run(parsed.task);
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
    const health = runHealth(store.path(sessionId));
    io.stdout.write(`${JSON.stringify(health)}\n`);
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

type FlagName =
  | "print"
  | "provider"
  | "model"
  | "scorer"
  | "ladder"
  | "base-url"
  | "api-key-env";

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
      scorer?: "rule" | "model" | "jev";
      ladder?: { provider: string; model: string }[];
    }
  | { ok: false; message: string };

export function parseArgs(argv: string[]): Parsed {
  let task: string | undefined;
  let provider: string | undefined;
  let model: string | undefined;
  let baseUrl: string | undefined;
  let apiKeyEnv: string | undefined;
  let passthrough = false;
  let json = false;
  let scorer: "rule" | "model" | "jev" | undefined;
  let ladder: { provider: string; model: string }[] | undefined;
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
        if (
          taken.value !== "rule" &&
          taken.value !== "model" &&
          taken.value !== "jev"
        ) {
          return { ok: false, message: usage() };
        }
        scorer = taken.value;
      }
      if (taken.name === "ladder") {
        const parsed = parseLadder(taken.value);
        if (!parsed) return { ok: false, message: usage() };
        ladder = parsed;
      }
      if (taken.name === "base-url") baseUrl = taken.value;
      if (taken.name === "api-key-env") apiKeyEnv = taken.value;
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
    ladder,
  };
}

function usage(): string {
  return [
    "usage: sage [-p | --print] <task> [--provider <preset>] [--model <id>] [--base-url <url>] [--api-key-env <NAME>] [--ladder provider/model,provider/model] [--passthrough] [--scorer rule|model|jev] [--json]",
    "       sage stats <session.jsonl>",
    "       sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] --out <dir>",
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
  if (arg === "--ladder") return "ladder";
  if (arg === "--base-url") return "base-url";
  if (arg === "--api-key-env") return "api-key-env";
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
    "ladder",
    "base-url",
    "api-key-env",
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
    if (env.ANTHROPIC_API_KEY) selected = "anthropic";
    else if (env.OPENAI_API_KEY) selected = "openai";
    else {
      return {
        ok: false,
        exitCode: 1,
        message: "Missing ANTHROPIC_API_KEY or OPENAI_API_KEY",
      };
    }
  }
  if (selected === "typesafe") {
    return {
      ok: false,
      exitCode: 1,
      message:
        "typesafe is not a chat endpoint; use --scorer jev with TYPESAFE_API_KEY",
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
