/**
 * Assembles the coding agent: AgentSession, its tools, its view, and its
 * route policy.
 *
 * Contract: the session gets the seven first-party tools plus
 * update_ledger, expand, spawn_subtask, and spawn_subtasks, and logs to a
 * JsonlLogStore under `<cwd>/.sage/sessions` unless a store is given.
 * Subtask presets load from `<cwd>/.sage/subtasks` and, when `home` is set,
 * from there too. The view is contextCompiler with
 * {@link DEFAULT_COMPILER_BUDGET},
 * the four first-party claims, and {@link defaultRoutePolicy}, unless
 * `passthrough` selects passthroughView. Subtasks always compile.
 *
 * Intentionally not in the Log: the store location, the sandbox settings,
 * and the route ladder. The system prompt is model-visible and is stored on
 * request_header; claim text reaches the Log only through the manifest the
 * kernel appends.
 */

import { randomUUID } from "node:crypto";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  AgentSession,
  JsonlLogStore,
  type LogStore,
  passthroughView,
  type ViewProvider,
} from "@sage/core";
import {
  agentsMd,
  type ClaimProvider,
  contextCompiler,
  costTierPolicy,
  DEFAULT_COMPILER_BUDGET,
  expandTool,
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
  listModels,
  type ModelRef,
  type ProviderAdapter,
  type Sampling,
} from "@sage/provider";
import {
  type BashExec,
  createTools,
  SAGE_DIR_NAME,
  type ToolSandbox,
} from "@sage/tools";
import { BENCH_WORKSPACE_PREFIX } from "./bench/types.js";
import type { LadderRung, ScorerApply } from "./options.js";

/** Where a session's Log lives, relative to its cwd. */
export const SESSION_LOG_SUBDIR = path.join(SAGE_DIR_NAME, "sessions");

/** Paths the route policy keeps on the home provider. */
const SENSITIVE = /(^|\/)\.env$|\/secrets\//;

/** A session plus the store it writes and its id. The caller owns all three. */
export type CodingSession = {
  session: AgentSession;
  store: LogStore;
  sessionId: string;
};

/** Inputs to {@link createCodingSession}. One of `adapter` / `adapters` is required. */
export type CodingSessionOptions = {
  cwd: string;
  /** Adapter for `model.provider` alone. Ignored when `adapters` is set. */
  adapter?: ProviderAdapter;
  adapters?: Record<string, ProviderAdapter>;
  model: ModelRef;
  /** Defaults to {@link systemPrompt} for `cwd`. */
  system?: string;
  sessionId?: string;
  /** Absolute; defaults to {@link SESSION_LOG_SUBDIR} under `cwd`. */
  sessionsDir?: string;
  store?: LogStore;
  /** Test seam. Production bash spawns real processes. */
  exec?: BashExec;
  sandbox?: Partial<ToolSandbox>;
  /** Realpath of the temp root passed into the bash sandbox. */
  tmpRoot?: string;
  /** Bench workspace prefix passed into the bash sandbox. */
  benchPrefix?: string;
  /** Test seam. Production uses process.platform. */
  platform?: NodeJS.Platform;
  /** Test seam. Production probes `unshare -n` on linux. */
  unshareAvailable?: boolean;
  sampling?: Sampling;
  /** Root compiler budget in tokens; defaults to {@link DEFAULT_COMPILER_BUDGET}. */
  budget?: number;
  passthrough?: boolean;
  scorer?: Scorer;
  shadowScorer?: Scorer;
  scorerApply?: ScorerApply;
  ladder?: readonly LadderRung[];
  /** Home dir for user-level subtask presets. */
  home?: string;
};

/**
 * Builds the coding session. Throws when neither `adapter` nor `adapters`
 * is given, or no adapter serves `model.provider`, and when the sandbox
 * cannot be enforced on this platform (from `createTools`).
 */
export function createCodingSession(opts: CodingSessionOptions): CodingSession {
  const adapters = requireAdapters(opts);
  const sessionId = opts.sessionId ?? randomUUID();
  const store =
    opts.store ??
    new JsonlLogStore({
      dir: opts.sessionsDir ?? path.join(opts.cwd, SESSION_LOG_SUBDIR),
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
    ...firstPartyTools(opts),
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
    viewProvider: rootView(opts, claims, route),
    sampling: opts.sampling,
  });
  return { session, store, sessionId };
}

/**
 * Route policy for the root and subtask compilers. The ladder is `ladder`
 * when given, else only the current model, and the cheapest allowed rung
 * wins. When the view touches `.env` or `secrets/`, every provider other
 * than `home` is denied.
 */
export function defaultRoutePolicy(
  providers: readonly string[],
  home: string,
  ladder?: readonly LadderRung[],
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

/** The root system prompt. Model-visible; changing it changes every request. */
export function systemPrompt(cwd: string): string {
  return [
    "You are sage, a coding agent.",
    "Complete the task in the workspace using the provided tools.",
    "Read before editing. Use edit for a single exact replacement.",
    "Stop when the task is done.",
    `Workspace: ${cwd}`,
  ].join("\n");
}

function requireAdapters(
  opts: CodingSessionOptions,
): Record<string, ProviderAdapter> {
  const provider = opts.model.provider;
  if (opts.adapters) {
    if (!opts.adapters[provider]) {
      throw new Error(`No adapter for provider: ${provider}`);
    }
    return opts.adapters;
  }
  if (opts.adapter) return { [provider]: opts.adapter };
  throw new Error("createCodingSession requires adapter or adapters");
}

/** passthroughView, or the root compiler with the session's scorers. */
function rootView(
  opts: CodingSessionOptions,
  claims: ClaimProvider[],
  route: RoutePolicy,
): ViewProvider {
  if (opts.passthrough) return passthroughView;
  return contextCompiler({
    budget: opts.budget ?? DEFAULT_COMPILER_BUDGET,
    claims,
    route,
    ...(opts.scorer ? { scorer: opts.scorer } : {}),
    ...(opts.shadowScorer ? { shadowScorer: opts.shadowScorer } : {}),
    ...(opts.scorerApply ? { scorerApply: opts.scorerApply } : {}),
  });
}

function firstPartyTools(opts: CodingSessionOptions) {
  return createTools({
    cwd: opts.cwd,
    exec: opts.exec,
    sandbox: opts.sandbox,
    platform: opts.platform,
    tmpRoot: opts.tmpRoot ?? realpathSync(tmpdir()),
    benchPrefix: opts.benchPrefix ?? BENCH_WORKSPACE_PREFIX,
    ...(opts.unshareAvailable !== undefined
      ? { unshareAvailable: opts.unshareAvailable }
      : {}),
  });
}
