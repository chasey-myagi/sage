/**
 * One-shot coding CLI.
 *
 * Contract: parse argv, assemble AgentSession with the seven tools plus
 * update_ledger, expand, spawn_subtask, and spawn_subtasks, and a
 * JsonlLogStore under
 * `<cwd>/.sage/sessions`. The view
 * is contextCompiler with budget 40000, the four first-party claims, and
 * the default route policy, unless `--passthrough` selects passthroughView.
 * Two API keys assemble two adapters. One key routes only inside that
 * provider. stdout is one JSON
 * RunSummary. The kernel appends each turn's manifest. Exit 0 when the
 * outcome is done, otherwise 1.
 *
 * Intentionally not in the Log: argv, API keys, and the stderr progress
 * lines. The system prompt is model-visible and is stored on request_header.
 * Claim text reaches the Log only through the manifest the kernel appends.
 */

import { randomUUID } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  AgentSession,
  JsonlLogStore,
  type LogStore,
  passthroughView,
} from "@sage/core";
import {
  agentsMd,
  contextCompiler,
  costTierPolicy,
  expandTool,
  progressLedger,
  type RoutePolicy,
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
  type ProviderAdapter,
  type Sampling,
} from "@sage/provider";
import { type BashExec, createTools } from "@sage/tools";

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
  const route = defaultRoutePolicy(Object.keys(adapters), opts.model.provider);
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
};

export async function runCli(
  argv: string[],
  io: Partial<CliIo> = {},
): Promise<number> {
  const stdout = io.stdout ?? process.stdout;
  const stderr = io.stderr ?? process.stderr;
  const cwd = io.cwd ?? process.cwd();
  const env = io.env ?? process.env;
  const parsed = parseArgs(argv);
  if (!parsed.ok) {
    stderr.write(`${parsed.message}\n`);
    return 1;
  }
  const auth = resolveAuth(parsed.provider, parsed.model, env);
  if (!auth.ok) {
    stderr.write(`${auth.message}\n`);
    return 1;
  }
  const adapters =
    io.adapters ??
    (io.adapter ? { [auth.provider]: io.adapter } : adaptersFromEnv(env));
  if (!adapters[auth.provider]) {
    adapters[auth.provider] = createAdapter(auth.provider, auth.apiKey);
  }
  const { session } = createCodingSession({
    cwd,
    adapters,
    model: { provider: auth.provider, model: auth.model },
    passthrough: parsed.passthrough,
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
): RoutePolicy {
  const unique = [...new Set(providers)];
  const candidates = unique.flatMap((provider) => [...listModels(provider)]);
  const ladder = candidates.map((entry) => ({
    provider: entry.provider,
    model: entry.model,
  }));
  const deny = unique.filter((provider) => provider !== home);
  const policy = withSensitiveFilter(
    costTierPolicy({ ladder, pick: "cheapest-allowed" }),
    sensitivePolicy({
      rules:
        deny.length === 0 ? [] : [{ pattern: SENSITIVE, denyProviders: deny }],
    }),
  );
  return (input) => policy({ ...input, candidates });
}

export function adaptersFromEnv(
  env: NodeJS.ProcessEnv,
): Record<string, ProviderAdapter> {
  const adapters: Record<string, ProviderAdapter> = {};
  if (env.ANTHROPIC_API_KEY) {
    adapters.anthropic = new AnthropicAdapter({
      apiKey: env.ANTHROPIC_API_KEY,
    });
  }
  if (env.OPENAI_API_KEY) {
    adapters.openai = new OpenAIAdapter({ apiKey: env.OPENAI_API_KEY });
  }
  return adapters;
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

type Parsed =
  | {
      ok: true;
      task: string;
      provider?: string;
      model?: string;
      passthrough: boolean;
    }
  | { ok: false; message: string };

export function parseArgs(argv: string[]): Parsed {
  let task: string | undefined;
  let provider: string | undefined;
  let model: string | undefined;
  let passthrough = false;
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
    const taken = take(argv, index, arg);
    if (taken.kind === "unknown") return { ok: false, message: taken.message };
    if (taken.kind === "flag") {
      index = taken.next;
      if (taken.name === "print") task = taken.value;
      if (taken.name === "provider") provider = taken.value;
      if (taken.name === "model") model = taken.value;
      continue;
    }
    positional.push(arg);
  }
  if (task !== undefined && positional.length > 0) {
    return { ok: false, message: usage() };
  }
  if (task === undefined) task = positional.join(" ");
  if (!task) return { ok: false, message: usage() };
  return { ok: true, task, provider, model, passthrough };
}

function usage(): string {
  return "usage: sage [-p | --print] <task> [--provider anthropic|openai] [--model <id>] [--passthrough]";
}

function take(
  argv: string[],
  index: number,
  arg: string,
):
  | {
      kind: "flag";
      name: "print" | "provider" | "model";
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

function flagName(
  arg: string,
): "print" | "provider" | "model" | "unknown" | undefined {
  if (arg === "--print" || arg === "-p") return "print";
  if (arg === "--provider") return "provider";
  if (arg === "--model") return "model";
  if (arg.startsWith("-")) return "unknown";
  return undefined;
}

function inlineFlag(
  arg: string,
): { name: "print" | "provider" | "model"; value: string } | undefined {
  const names = ["print", "provider", "model"] as const;
  for (const name of names) {
    const prefix = `--${name}=`;
    if (!arg.startsWith(prefix)) continue;
    const value = arg.slice(prefix.length);
    if (!value) return undefined;
    return { name, value };
  }
  return undefined;
}

function resolveAuth(
  provider: string | undefined,
  model: string | undefined,
  env: NodeJS.ProcessEnv,
):
  | { ok: true; provider: string; model: string; apiKey: string }
  | { ok: false; message: string } {
  let selected = provider;
  if (!selected) {
    if (env.ANTHROPIC_API_KEY) selected = "anthropic";
    else if (env.OPENAI_API_KEY) selected = "openai";
    else {
      return {
        ok: false,
        message: "Missing ANTHROPIC_API_KEY or OPENAI_API_KEY",
      };
    }
  }
  if (selected !== "anthropic" && selected !== "openai") {
    return { ok: false, message: `Unknown provider: ${selected}` };
  }
  const apiKey =
    selected === "anthropic" ? env.ANTHROPIC_API_KEY : env.OPENAI_API_KEY;
  if (!apiKey) {
    const name =
      selected === "anthropic" ? "ANTHROPIC_API_KEY" : "OPENAI_API_KEY";
    return { ok: false, message: `Missing ${name}` };
  }
  return {
    ok: true,
    provider: selected,
    model: model ?? defaultModel(selected).model,
    apiKey,
  };
}

function createAdapter(provider: string, apiKey: string): ProviderAdapter {
  if (provider === "openai") return new OpenAIAdapter({ apiKey });
  return new AnthropicAdapter({ apiKey });
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
