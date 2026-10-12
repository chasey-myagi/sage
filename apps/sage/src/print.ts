/**
 * `sage [-p] <task>`: one run of the coding agent in the current directory.
 *
 * Contract: every check that can fail runs before the session is built, in
 * this order: argv, `--scorer-apply entry` without a model or jev scorer
 * (exit 2), sandbox support on this platform (exit 2), the Jev key when a
 * jev scorer is named (exit 2), the user providers file (exit 2), then
 * provider auth (exit 1 or 2). stdout
 * is one JSON RunSummary line, including `health`; exit 0 when the outcome
 * is `done`, otherwise 1. stderr gets one progress line per model call.
 * Flag semantics are in `apps/sage/README.md`.
 *
 * Intentionally not in the Log: argv, API keys, the stderr progress lines,
 * and the stderr note that `--passthrough` ignored `--budget`. The tool
 * network setting is argv; bash reports `network: "none"` through the
 * kernel's toolResults, not as its own entry. `SAGE_SANDBOX_ALLOW_ROOT`
 * and the sibling-workspace deny are not Log entries.
 */

import { type AgentSession, errorText } from "@sage/core";
import type { LocalProviders, ProviderAdapter } from "@sage/provider";
import { inputTokens } from "@sage/provider/contract";
import {
  resolveToolSandbox,
  SANDBOX_ALLOW_ROOT_ENV,
  type ToolSandbox,
  toolSandboxSupport,
} from "@sage/tools";
import { type PrintOptions, parseArgs } from "./args.js";
import { adaptersFromEnv, createAdapter, resolveAuth } from "./auth.js";
import { createCodingSession } from "./coding-session.js";
import type { TextOut } from "./command-io.js";
import type { CliIo } from "./io.js";
import { loadLocalProviders } from "./local-providers.js";
import { jevApiKey, namedScorer } from "./scorers.js";

type SandboxPlan = {
  sandbox: ToolSandbox;
  platform: NodeJS.Platform;
  unshareAvailable?: boolean;
};

/** Runs `sage -p` and returns the process exit code. */
export async function runPrint(argv: string[], io: CliIo): Promise<number> {
  const { env, stderr } = io;
  const parsed = parseArgs(argv);
  if (!parsed.ok) {
    stderr.write(`${parsed.message}\n`);
    return parsed.exitCode;
  }
  const plan = preflight(parsed, io);
  if (typeof plan === "string") {
    stderr.write(`${plan}\n`);
    return 2;
  }
  const loaded = loadLocalProviders(env);
  if (!loaded.ok) {
    stderr.write(`${loaded.message}\n`);
    return 2;
  }
  const { home, local } = loaded;
  const auth = resolveAuth(parsed, env, local);
  if (!auth.ok) {
    stderr.write(`${auth.message}\n`);
    return auth.exitCode;
  }
  const adapters = assembleAdapters(io, auth, local);
  if (parsed.passthrough && parsed.budget !== undefined) {
    stderr.write(
      "Ignoring --budget because --passthrough does not compile a context\n",
    );
  }
  const model = { provider: auth.provider, model: auth.model };
  const { session } = createCodingSession({
    cwd: io.cwd,
    adapters,
    model,
    passthrough: parsed.passthrough,
    home,
    localModels: local.models,
    sandbox: plan.sandbox,
    platform: plan.platform,
    ...(plan.unshareAvailable !== undefined
      ? { unshareAvailable: plan.unshareAvailable }
      : {}),
    ...(io.exec ? { exec: io.exec } : {}),
    ...(parsed.budget !== undefined && !parsed.passthrough
      ? { budget: parsed.budget }
      : {}),
    ...(parsed.ladder ? { ladder: parsed.ladder } : {}),
    scorer: namedScorer(parsed.scorer, adapters, model, env, io.jevFetch),
    shadowScorer: namedScorer(
      parsed.shadowScorer,
      adapters,
      model,
      env,
      io.jevFetch,
    ),
    ...(parsed.scorerApply ? { scorerApply: parsed.scorerApply } : {}),
  });
  reportProgress(session, stderr);
  return runSession(session, parsed, io);
}

/**
 * Checks that need no provider: entry scoring, sandbox support, Jev key.
 * Returns the stderr line for an exit-2 failure, else the sandbox plan.
 */
function preflight(parsed: PrintOptions, io: CliIo): string | SandboxPlan {
  if (
    parsed.scorerApply === "entry" &&
    parsed.scorer !== "model" &&
    parsed.scorer !== "jev"
  ) {
    return "entry scoring needs --scorer model|jev";
  }
  const plan = planSandbox(parsed, io);
  if (typeof plan === "string") return plan;
  if (
    (parsed.scorer === "jev" || parsed.shadowScorer === "jev") &&
    !jevApiKey(io.env)
  ) {
    return "Missing JEV_API_KEY or TYPESAFE_API_KEY";
  }
  return plan;
}

/**
 * Resolves the bash sandbox and refuses a network cut this platform cannot
 * enforce. On linux a supported `none` means `unshare -n` was found.
 */
function planSandbox(parsed: PrintOptions, io: CliIo): string | SandboxPlan {
  const allowRoot = io.env[SANDBOX_ALLOW_ROOT_ENV];
  const sandbox = resolveToolSandbox({
    network: parsed.toolNetwork ?? "full",
    ...(typeof allowRoot === "string" && allowRoot.length > 0
      ? { allowRoot }
      : {}),
  });
  const platform = io.platform ?? process.platform;
  let unshareAvailable = io.unshareAvailable;
  if (sandbox.network === "none" || sandbox.fsIsolate) {
    const support = toolSandboxSupport({
      platform,
      network: sandbox.network,
      fsIsolate: sandbox.fsIsolate,
      ...(unshareAvailable !== undefined ? { unshareAvailable } : {}),
    });
    if (!support.ok) return support.message;
    if (platform === "linux" && sandbox.network === "none") {
      unshareAvailable = true;
    }
  }
  return {
    sandbox,
    platform,
    ...(unshareAvailable !== undefined ? { unshareAvailable } : {}),
  };
}

/**
 * Injected adapters are used as given; otherwise one adapter per keyed
 * preset, plus the selected provider's adapter built from the resolved key
 * and base URL. Throws when an injected map does not serve the selected
 * provider: injection is a test seam, and filling the gap with a real
 * adapter would send the test to the network.
 */
function assembleAdapters(
  io: CliIo,
  auth: { provider: string; apiKey: string; baseUrl: string },
  local: LocalProviders,
): Record<string, ProviderAdapter> {
  if (io.adapters) {
    if (!io.adapters[auth.provider]) {
      throw new Error(
        `Injected adapters do not serve the selected provider: ${auth.provider}`,
      );
    }
    return io.adapters;
  }
  const adapters = adaptersFromEnv(io.env, local);
  adapters[auth.provider] = createAdapter(
    auth.provider,
    auth.apiKey,
    auth.baseUrl,
    local,
  );
  return adapters;
}

/** Writes `turn N · tokens in/out · tool calls` to stderr after each model call. */
function reportProgress(session: AgentSession, stderr: TextOut): void {
  let turn = 0;
  session.hooks.observe("turn_start", (payload) => {
    turn = payload.turn;
  });
  session.hooks.observe("llm_end", ({ assistant }) => {
    const usage = assistant.usage;
    const input = inputTokens(usage);
    const calls = assistant.content.filter(
      (block) => block.type === "tool_use",
    ).length;
    stderr.write(
      `turn ${turn} · tokens ${input}/${usage.output} · tool calls ${calls}\n`,
    );
  });
}

async function runSession(
  session: AgentSession,
  parsed: PrintOptions,
  io: CliIo,
): Promise<number> {
  try {
    const summary = await session.run(
      parsed.task,
      parsed.maxTurns !== undefined ? { maxTurns: parsed.maxTurns } : undefined,
    );
    io.stdout.write(`${JSON.stringify(summary)}\n`);
    return summary.outcome === "done" ? 0 : 1;
  } catch (error) {
    io.stderr.write(`${errorText(error)}\n`);
    return 1;
  }
}
