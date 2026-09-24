/**
 * Agent loop. One `AgentSession` drives turns until the model stops, the
 * run is aborted, a turn fails, or `maxTurns` runs out.
 *
 * Guarantees to callers:
 * - Everything a request carries is in the Log before the request is sent:
 *   message entries plus that turn's request_header. The manifest entry is
 *   the kernel's record of the view that produced the request.
 * - The kernel does not compress, drop, or rewrite the ViewProvider's view.
 *   It checks tool pairing and the frozen head, and sends nothing otherwise.
 * - A tool that throws becomes an error tool_result and the turn continues.
 *   Any other throw appends a terminal entry, then propagates.
 * - Once the user message is appended, every run ends with one terminal
 *   entry. Hooks never receive the store.
 *
 * The per-turn order is documented on `turn()` and the methods it calls.
 *
 * Intentionally not in the Log: the hook list, the abort signal, observer
 * errors, the adapter table, the sleep function, and assistant text
 * discarded by a turn retry. `turn_retry` is persisted and is not
 * model-visible.
 */

import {
  type AssistantMessage,
  addUsage,
  emptyUsage,
  type Head,
  type Message,
  type ModelRef,
  type PreparedCall,
  type ProviderAdapter,
  type Request,
  type Sampling,
  type Sleeper,
  sleep,
  type ToolResultBlock,
  type Usage,
} from "@sage/provider/contract";
import { type RunHealth, runHealth } from "./health.js";
import { HookDispatcher } from "./hooks/dispatcher.js";
import type { Hooks } from "./hooks/types.js";
import type { RequestHeaderPayload } from "./log/kinds.js";
import { lastRequestHeader, latestViewEpoch } from "./log/path.js";
import type { LogStore } from "./log/store.js";
import type { EntryId, StoredEntry } from "./log/types.js";
import { runToolCalls, type ToolResultDetails, toolUses } from "./tool-run.js";
import type { HarnessTool } from "./tools.js";
import { collectWithTurnRetry } from "./turn-retry.js";
import {
  assertHeadFrozenOnPath,
  assertPaired,
  systemHash,
  toolsHash,
} from "./view/assert.js";
import { passthroughView } from "./view/passthrough.js";
import type { CompiledView, ViewProvider } from "./view/types.js";

/** How one turn ended. `max_turns` is only ever a run outcome. */
export type TurnOutcome =
  | "continue"
  | "done"
  | "error"
  | "aborted"
  | "max_turns";

/**
 * Result of `AgentSession.run`. The same `outcome`, `usage`, `health`, and
 * `error` are stored on the terminal entry.
 */
export type RunSummary = {
  outcome: Exclude<TurnOutcome, "continue">;
  turns: number;
  /** Sum of this run's assistant usage. Child session usage is not added. */
  usage: Usage;
  /** model_change entries appended by this run. */
  modelChanges: number;
  /** Observer throws, in order. Reset at the start of each run. */
  observerErrors: { event: string; error: string }[];
  /** `runHealth` of the path at finish. */
  health: RunHealth;
  error?: string;
};

/** Turn cap `AgentSession.run` uses when the caller passes no `maxTurns`. */
export const DEFAULT_MAX_TURNS = 80;

type TurnResult =
  | { kind: "continue"; usage: Usage; prepared: PreparedCall }
  | {
      kind: "done" | "aborted" | "error";
      usage: Usage;
      error?: string;
      prepared: PreparedCall;
    };

function sameModel(left: ModelRef, right: ModelRef): boolean {
  return left.provider === right.provider && left.model === right.model;
}

/**
 * `model_change.reason`: `route:` plus the text after the last `;route:` in
 * `epochDecision.reason`; a reason that already starts with `route:` as is;
 * otherwise bare `route:`.
 */
function routeReason(view: CompiledView): string {
  const reason = view.manifest.epochDecision?.reason ?? "";
  const marker = ";route:";
  const at = reason.lastIndexOf(marker);
  if (at >= 0) return `route:${reason.slice(at + marker.length)}`;
  if (reason.startsWith("route:")) return reason;
  return "route:";
}

/** Adapter defaults, overridden field by field by the session's sampling. */
function mergeSampling(
  defaults: Record<string, unknown>,
  sampling: Sampling | undefined,
): Sampling {
  const merged: Sampling = {};
  if (typeof defaults.maxTokens === "number")
    merged.maxTokens = defaults.maxTokens;
  if (typeof defaults.temperature === "number") {
    merged.temperature = defaults.temperature;
  }
  if (sampling?.maxTokens !== undefined) merged.maxTokens = sampling.maxTokens;
  if (sampling?.temperature !== undefined)
    merged.temperature = sampling.temperature;
  return merged;
}

/**
 * The agent loop over one Log session. Not safe for overlapping `run` calls:
 * appends on one session must be serial.
 */
export class AgentSession {
  /** Registration surface for observers, tool hooks, guards, and arounds. */
  readonly hooks: Hooks;
  private readonly dispatcher: HookDispatcher;
  private readonly observerErrors: { event: string; error: string }[] = [];
  private readonly store: LogStore;
  private readonly sessionId: string;
  private readonly adapters: Record<string, ProviderAdapter>;
  private adapter: ProviderAdapter;
  private model: ModelRef;
  private modelChanges = 0;
  private readonly system: string;
  private readonly tools: HarnessTool[];
  private readonly viewProvider: ViewProvider;
  private readonly sampling: Sampling | undefined;
  private readonly sleep: Sleeper;
  private readonly parent?: {
    sessionId: string;
    leafId: EntryId;
    depth: number;
  };

  /**
   * Takes `adapters` (provider name → adapter), or one `adapter`, which is
   * stored under the starting model's provider unless `adapters` already has
   * one there. Throws when no adapter serves the starting provider.
   *
   * `viewProvider` defaults to the passthrough view. `sleep` replaces the
   * wait between turn retries (test seam; production uses a real timer).
   * `parent`, when set, marks a child session: each turn snapshots
   * `store.path(parent.sessionId, parent.leafId)` onto
   * `ViewContext.parentPath` and sets `ViewContext.depth`. The kernel does
   * not copy those entries onto this session.
   */
  constructor(
    opts: {
      store: LogStore;
      sessionId: string;
      model: ModelRef;
      system: string;
      tools: HarnessTool[];
      viewProvider?: ViewProvider;
      sampling?: Sampling;
      sleep?: Sleeper;
      parent?: { sessionId: string; leafId: EntryId; depth: number };
    } & (
      | { adapter: ProviderAdapter; adapters?: Record<string, ProviderAdapter> }
      | { adapters: Record<string, ProviderAdapter>; adapter?: ProviderAdapter }
    ),
  ) {
    this.store = opts.store;
    this.sessionId = opts.sessionId;
    const adapters: Record<string, ProviderAdapter> = {
      ...(opts.adapters ?? {}),
    };
    if (opts.adapter && adapters[opts.model.provider] === undefined) {
      adapters[opts.model.provider] = opts.adapter;
    }
    const selected = adapters[opts.model.provider];
    if (!selected) {
      throw new Error(`No adapter for provider: ${opts.model.provider}`);
    }
    this.adapters = adapters;
    this.adapter = selected;
    this.model = { provider: opts.model.provider, model: opts.model.model };
    this.system = opts.system;
    this.tools = opts.tools;
    this.viewProvider = opts.viewProvider ?? passthroughView;
    this.sampling = opts.sampling;
    this.sleep = opts.sleep ?? sleep;
    this.parent = opts.parent;
    this.dispatcher = new HookDispatcher(this.observerErrors);
    this.hooks = this.dispatcher;
  }

  /**
   * Appends `userText` as a user message, then runs turns until one ends the
   * run. `prepare` runs here and again on each model change.
   *
   * Ends with a terminal entry and `session_end`: `done`; `aborted` (checked
   * before each turn and inside it); `error` (no adapter for a routed
   * provider, a non-transport finish error, or a transport error that
   * outlasted the whole-turn retries); or `max_turns` once `maxTurns` turns ran
   * (default 80). A throw that is not a tool failure, such as a seam
   * assertion or a store error, writes a terminal `error` entry and is
   * rethrown.
   */
  async run(
    userText: string,
    opts?: { signal?: AbortSignal; maxTurns?: number },
  ): Promise<RunSummary> {
    const signal = opts?.signal;
    const maxTurns = opts?.maxTurns ?? DEFAULT_MAX_TURNS;
    this.observerErrors.length = 0;
    this.modelChanges = 0;
    let prepared = this.adapter.prepare(this.model);
    await this.dispatcher.emit("session_start", { sessionId: this.sessionId });
    this.appendUser(userText);

    let turns = 0;
    let usage: Usage = emptyUsage();
    try {
      while (turns < maxTurns) {
        if (signal?.aborted) {
          return await this.finish("aborted", turns, usage);
        }
        turns += 1;
        const outcome = await this.turn(turns, prepared, signal);
        prepared = outcome.prepared;
        usage = addUsage(usage, outcome.usage);
        if (outcome.kind === "continue") continue;
        return await this.finish(outcome.kind, turns, usage, outcome.error);
      }
      return await this.finish("max_turns", turns, usage);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await this.finish("error", turns, usage, message);
      throw error;
    }
  }

  /**
   * One turn, in order: `turn_start`; `compileView`; `applyRoute` (a
   * missing adapter ends the turn as `error` before any request entry);
   * `recordRequest`; `collectWithTurnRetry`, which streams the request and
   * may resend it after a transport error; `recordAssistant`; then
   * `settleAssistant` picks the outcome. `turn_end` carries that outcome.
   * The result's `prepared` is the call the next turn uses.
   */
  private async turn(
    turn: number,
    prepared: PreparedCall,
    signal: AbortSignal | undefined,
  ): Promise<TurnResult> {
    await this.dispatcher.emit("turn_start", { turn });
    const view = await this.compileView(turn, prepared);
    const routed = this.applyRoute(view, prepared);
    if ("error" in routed) {
      return this.endTurn(turn, {
        kind: "error",
        usage: emptyUsage(),
        error: routed.error,
        prepared,
      });
    }
    const active = routed.prepared;
    const request = this.recordRequest(view, active);
    const streamed = await collectWithTurnRetry({
      store: this.store,
      sessionId: this.sessionId,
      adapter: this.adapter,
      sleep: this.sleep,
      turn,
      request,
      prepared: active,
      signal,
    });
    if (streamed === "aborted") {
      return this.endTurn(turn, {
        kind: "aborted",
        usage: emptyUsage(),
        prepared: active,
      });
    }
    await this.recordAssistant(streamed);
    return this.endTurn(
      turn,
      await this.settleAssistant(streamed, active, signal),
    );
  }

  private async endTurn(turn: number, result: TurnResult): Promise<TurnResult> {
    await this.dispatcher.emit("turn_end", { turn, outcome: result.kind });
    return result;
  }

  /**
   * Asks the ViewProvider for this turn's view and checks it before anything
   * is appended. `ViewContext.path` is a structured clone of the path taken
   * before `view()`; `tools` is the constructor's tool list; `head` is the
   * session's system prompt and tool schemas. Throws `UnpairedToolCallError`
   * or `HeadChangedMidEpochError` (see `assertHeadFrozenOnPath`). Observers of
   * `view_compiled` receive a structured clone, so their edits never reach
   * the request.
   */
  private async compileView(
    turn: number,
    prepared: PreparedCall,
  ): Promise<CompiledView> {
    const parentPath = this.parentSnapshot();
    const view = await this.viewProvider.view({
      sessionId: this.sessionId,
      turn,
      path: structuredClone(this.store.path(this.sessionId)),
      model: this.model,
      prepared,
      head: this.injectedHead(),
      tools: this.tools,
      depth: this.parent?.depth ?? 0,
      ...(parentPath ? { parentPath } : {}),
    });
    assertPaired(view.messages);
    assertHeadFrozenOnPath(view, this.store.path(this.sessionId));
    await this.dispatcher.emit("view_compiled", {
      view: structuredClone(view),
    });
    return view;
  }

  /**
   * Appends this turn's request entries in order (`view_epoch` when due,
   * `request_header`, `manifest`) and returns the request they describe.
   * The manifest payload is `view.manifest`; its details are `view.details`.
   */
  private recordRequest(view: CompiledView, active: PreparedCall): Request {
    const sampling = mergeSampling(active.adapterDefaults, this.sampling);
    this.appendViewEpoch(view);
    this.appendHeader(view, sampling, active);
    this.store.append(this.sessionId, {
      kind: "manifest",
      payload: view.manifest,
      ...(view.details ? { details: view.details } : {}),
    });
    return {
      model: this.model,
      head: view.head,
      messages: view.messages,
      cacheBreakpoints: [...view.cacheBreakpoints],
      sampling,
    };
  }

  /**
   * Appends the assistant message, then emits `llm_end`. Details are
   * `{responseModel, responseId, api, usage, finish}`; the response id and
   * model come from the stream's response chunk and are empty when the call
   * failed before the body began.
   */
  private async recordAssistant(assistant: AssistantMessage): Promise<void> {
    this.store.append(this.sessionId, {
      kind: "message",
      payload: { role: "assistant", content: assistant.content },
      details: {
        responseModel: assistant.responseModel,
        responseId: assistant.responseId,
        api: assistant.api,
        usage: assistant.usage,
        finish: assistant.finish,
      },
    });
    await this.dispatcher.emit("llm_end", { assistant });
  }

  /**
   * Maps a recorded assistant message to the turn outcome. An aborted finish
   * or signal is `aborted`; a finish error is `error`; `tool_use` runs the
   * tools and appends one user message with every result, then `continue`
   * (or `aborted`, with nothing appended, when the signal fired meanwhile);
   * any other finish is `done`.
   */
  private async settleAssistant(
    assistant: AssistantMessage,
    active: PreparedCall,
    signal: AbortSignal | undefined,
  ): Promise<TurnResult> {
    const usage = assistant.usage;
    if (assistant.finish.kind === "aborted" || signal?.aborted) {
      return { kind: "aborted", usage, prepared: active };
    }
    if (assistant.finish.kind === "error") {
      return {
        kind: "error",
        usage,
        error: assistant.finish.error?.message ?? "model request failed",
        prepared: active,
      };
    }
    if (assistant.finish.kind !== "tool_use") {
      return { kind: "done", usage, prepared: active };
    }
    const ran = await runToolCalls({
      dispatcher: this.dispatcher,
      tools: this.tools,
      sessionId: this.sessionId,
      calls: toolUses(assistant.content),
      signal,
    });
    if (signal?.aborted) return { kind: "aborted", usage, prepared: active };
    this.appendToolResults(ran.results, ran.toolResults);
    return { kind: "continue", usage, prepared: active };
  }

  /**
   * Switches model when `view.epochStart.model` names a different one:
   * appends `model_change {from, to, reason}` first, then switches adapter
   * and prepares again. The head is not rewritten on a switch. Returns an
   * error, after the model_change entry, when no adapter serves the new
   * provider; no request is sent then.
   */
  private applyRoute(
    view: CompiledView,
    prepared: PreparedCall,
  ): { prepared: PreparedCall } | { error: string } {
    const next = view.epochStart?.model;
    if (!next || sameModel(next, this.model)) return { prepared };
    const from = { provider: this.model.provider, model: this.model.model };
    const to = { provider: next.provider, model: next.model };
    this.store.append(this.sessionId, {
      kind: "model_change",
      payload: { from, to, reason: routeReason(view) },
    });
    this.modelChanges += 1;
    const adapter = this.adapters[to.provider];
    if (!adapter) return { error: `No adapter for provider: ${to.provider}` };
    this.adapter = adapter;
    this.model = { provider: to.provider, model: to.model };
    return { prepared: adapter.prepare(this.model) };
  }

  private parentSnapshot(): StoredEntry[] | undefined {
    if (!this.parent) return undefined;
    return structuredClone(
      this.store.path(this.parent.sessionId, this.parent.leafId),
    );
  }

  private injectedHead(): Head {
    return {
      system: this.system,
      tools: this.tools.map((tool) => tool.def),
    };
  }

  private appendUser(text: string): void {
    const payload: Message = {
      role: "user",
      content: [{ type: "text", text }],
    };
    this.store.append(this.sessionId, { kind: "message", payload });
  }

  /**
   * Appends `view_epoch {epoch, reason}` when `view.epochStart` is set, or
   * when `view.epoch` is greater than the latest view_epoch on the path
   * (0 when there is none). `reason` is `epochStart.reason`, else
   * `epoch-advanced`. `details.epochHead` is the view's first message.
   */
  private appendViewEpoch(view: CompiledView): void {
    const latest = latestViewEpoch(this.store.path(this.sessionId));
    if (!view.epochStart && view.epoch <= latest) return;
    const firstMessage = view.messages[0];
    this.store.append(this.sessionId, {
      kind: "view_epoch",
      payload: {
        epoch: view.epoch,
        reason: view.epochStart?.reason ?? "epoch-advanced",
      },
      ...(firstMessage ? { details: { epochHead: firstMessage } } : {}),
    });
  }

  /**
   * Appends the request envelope. `system` and `tools` carry full text only
   * when their hash differs from the previous request_header on the path,
   * or when there is none; otherwise the hashes alone identify them.
   */
  private appendHeader(
    view: CompiledView,
    sampling: Sampling,
    prepared: PreparedCall,
  ): void {
    const prev = lastRequestHeader(this.store.path(this.sessionId));
    const systemPromptHash = systemHash(view.head.system);
    const toolsDigest = toolsHash(view.head.tools);
    const payload: RequestHeaderPayload = {
      provider: this.model.provider,
      model: this.model.model,
      systemPromptHash,
      toolsHash: toolsDigest,
      sampling: { ...sampling },
      adapterDefaults: { ...prepared.adapterDefaults },
    };
    if (!prev || prev.systemPromptHash !== systemPromptHash) {
      payload.system = view.head.system;
    }
    if (!prev || prev.toolsHash !== toolsDigest) {
      payload.tools = structuredClone(view.head.tools);
    }
    this.store.append(this.sessionId, { kind: "request_header", payload });
  }

  /**
   * Appends one user message holding every tool_result, with
   * `details.toolResults` when any tool returned details. Appends nothing
   * when there are no results.
   */
  private appendToolResults(
    results: ToolResultBlock[],
    toolResults: ToolResultDetails[],
  ): void {
    if (results.length === 0) return;
    const payload: Message = { role: "user", content: results };
    this.store.append(this.sessionId, {
      kind: "message",
      payload,
      ...(toolResults.length > 0 ? { details: { toolResults } } : {}),
    });
  }

  /**
   * Appends the terminal entry `{outcome, usage, health, error?}` and emits
   * `session_end`. `health` is `runHealth` of the path before the terminal
   * entry; the returned summary holds the same object.
   */
  private async finish(
    outcome: RunSummary["outcome"],
    turns: number,
    usage: Usage,
    error?: string,
  ): Promise<RunSummary> {
    const health = runHealth(this.store.path(this.sessionId));
    const summary: RunSummary = {
      outcome,
      turns,
      usage,
      modelChanges: this.modelChanges,
      observerErrors: this.observerErrors,
      health,
    };
    if (error !== undefined) summary.error = error;
    const payload: {
      outcome: string;
      usage: Usage;
      health: RunHealth;
      error?: string;
    } = {
      outcome,
      usage,
      health,
    };
    if (error !== undefined) payload.error = error;
    this.store.append(this.sessionId, { kind: "terminal", payload });
    await this.dispatcher.emit("session_end", { summary });
    return summary;
  }
}
