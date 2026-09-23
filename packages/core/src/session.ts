/**
 * Minimal agent loop.
 *
 * One turn, in order:
 * turn_start → view = await viewProvider.view(ctx) → assertPaired →
 * assertHeadFrozen → structuredClone(view) is what observe("view_compiled")
 * receives → if view.epochStart.model is set and differs from the current
 * model, append model_change {from, to, reason} then switch adapter and
 * prepare again (no adapter for that provider → terminal error, no request) →
 * if view.epochStart is set, or view.epoch is greater than the latest
 * view_epoch on the path (0 when none exists), append view_epoch
 * { epoch, reason } with details.epochHead = messages[0] →
 * append request_header (system and tools text only when that hash differs
 * from the previous request_header, or there is none) →
 * append manifest (payload is view.manifest, details is view.details) →
 * adapter.stream →
 * collect → when finish.error.transport is true (the provider sets it for
 * fetch and body-read failures; message text is not inspected) and this
 * turn has not yet appended an assistant message, wait (2s then 8s; `sleep` may replace
 * the wait) and, unless aborted during the wait, append turn_retry
 * {turn, attempt, reason} and stream the same request again, at most
 * twice →
 * append message(assistant) with details
 * {responseModel, responseId, api, usage, finish} (responseModel and
 * responseId are the stream's response chunk, empty when the call failed
 * before the body began) → llm_end →
 * if finish.kind is tool_use, run the tool pipeline for each tool_use and
 * append one user message of results (tool details on the entry, not the
 * block), then turn_end continue;
 * otherwise turn_end done.
 * abort, a non-transport finish error, or a transport error after two
 * retries appends terminal and stops. An HTTP status error is not resent
 * here: the provider already retried 429 and 5xx before the body began.
 * A throw from tool execute is a tool_result and the turn continues. Any
 * other throw appends terminal and stops.
 * Using up maxTurns appends terminal {outcome:"max_turns"}.
 * The default maxTurns is 80 when run is called without that option.
 *
 * Contract: the kernel does not compress. Hooks never receive the store.
 * ViewContext.path is a structured clone taken before view().
 * ViewContext.tools is the tool list given to the constructor.
 * prepare runs at the start of the run and again when the model changes.
 * model_change.reason is `route:` plus the text after `;route:` in
 * epochDecision.reason. Head bytes are not rewritten on a switch.
 * RunSummary.modelChanges counts model_change entries appended in this run.
 * RunSummary.health is runHealth of the path at finish, and the terminal
 * payload stores that same object. When a tool returns `details`, the user
 * message that carries the results stores `details.toolResults`
 * (`toolUseId`, `name`, `details`). Those fields are not on the
 * model-visible tool_result block.
 * Constructor takes `adapters` (provider → adapter) or one `adapter`, which
 * is stored under the starting model's provider.
 * `sleep`, when set, replaces the wait between turn retries.
 * `parent`, when set, snapshots `store.path(parent.sessionId, parent.leafId)`
 * onto `ViewContext.parentPath` and sets `ViewContext.depth`. The kernel
 * does not copy those entries onto this session. Child usage is not added
 * to this run's usage.
 *
 * Intentionally not in the Log: the hook list, the abort signal, observer
 * errors, the adapter table, the sleep function, and assistant text
 * discarded by a turn retry. Model-visible text is the message and
 * request_header entries. `turn_retry` is persisted and is not model-visible.
 * The manifest entry is the kernel's record of the view that produced that
 * request.
 */

import {
  type AssistantMessage,
  addUsage,
  backoffMs,
  type ContentBlock,
  collect,
  emptyUsage,
  type Head,
  isRecord,
  type Message,
  type ModelRef,
  type PreparedCall,
  type ProviderAdapter,
  type Request,
  type Sampling,
  type Sleeper,
  sleep,
  type ToolResultBlock,
  type ToolUseBlock,
  type Usage,
} from "@sage/provider/contract";
import { type RunHealth, runHealth } from "./health.js";
import { HookDispatcher } from "./hooks/dispatcher.js";
import type { Hooks } from "./hooks/types.js";
import { latestViewEpoch } from "./log/epoch.js";
import type { RequestHeaderPayload } from "./log/kinds.js";
import type { LogStore } from "./log/store.js";
import type { EntryId, StoredEntry } from "./log/types.js";
import type { HarnessTool } from "./tools.js";
import {
  assertHeadFrozen,
  assertPaired,
  systemHash,
  toolsHash,
} from "./view/assert.js";
import { HeadChangedMidEpochError } from "./view/errors.js";
import { passthroughView } from "./view/passthrough.js";
import type { CompiledView, ViewProvider } from "./view/types.js";

export type TurnOutcome =
  | "continue"
  | "done"
  | "error"
  | "aborted"
  | "max_turns";

export type RunSummary = {
  outcome: Exclude<TurnOutcome, "continue">;
  turns: number;
  usage: Usage;
  modelChanges: number;
  observerErrors: { event: string; error: string }[];
  health: RunHealth;
  error?: string;
};

/** Turn cap `AgentSession.run` uses when the caller passes no `maxTurns`. */
export const DEFAULT_MAX_TURNS = 80;

/** Whole-turn resends after a transport error, on top of the first call. */
const TURN_RETRY_LIMIT = 2;
/** First whole-turn retry delay; the shared curve makes the second 8s. */
const TURN_RETRY_BASE_MS = 2000;

function turnTransportReason(
  finish: AssistantMessage["finish"],
): string | undefined {
  if (finish.kind !== "error" || !finish.error) return undefined;
  return finish.error.transport === true ? finish.error.message : undefined;
}

/** Index of the last request_header on `path`, or -1. */
function lastHeaderIndex(path: readonly StoredEntry[]): number {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    if (path[index]?.kind === "request_header") return index;
  }
  return -1;
}

function assistantAfterLatestHeader(path: readonly StoredEntry[]): boolean {
  const header = lastHeaderIndex(path);
  if (header < 0) return false;
  for (const entry of path.slice(header + 1)) {
    if (entry.kind !== "message") continue;
    if (isRecord(entry.payload) && entry.payload.role === "assistant") {
      return true;
    }
  }
  return false;
}

function sameModel(left: ModelRef, right: ModelRef): boolean {
  return left.provider === right.provider && left.model === right.model;
}

function routeReason(view: CompiledView): string {
  const reason = view.manifest.epochDecision?.reason ?? "";
  const marker = ";route:";
  const at = reason.lastIndexOf(marker);
  if (at >= 0) return `route:${reason.slice(at + marker.length)}`;
  if (reason.startsWith("route:")) return reason;
  return "route:";
}

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

function asHeader(payload: unknown): RequestHeaderPayload | undefined {
  // The request_header validator already ran on append and on load.
  return isRecord(payload) ? (payload as RequestHeaderPayload) : undefined;
}

/**
 * `latest` is the newest view_epoch on the path. `frozen` is the newest one
 * at or before the last request_header, or `latest` when there is no header.
 */
function epochState(path: readonly StoredEntry[]): {
  latest: number;
  frozen: number;
} {
  const header = lastHeaderIndex(path);
  const latest = latestViewEpoch(path);
  const frozen =
    header < 0 ? latest : latestViewEpoch(path.slice(0, header + 1));
  return { latest, frozen };
}

function lastHeader(
  path: readonly StoredEntry[],
): RequestHeaderPayload | undefined {
  return asHeader(path[lastHeaderIndex(path)]?.payload);
}

function toolUses(content: ContentBlock[]): ToolUseBlock[] {
  return content.filter(
    (block): block is ToolUseBlock => block.type === "tool_use",
  );
}

export class AgentSession {
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

  private async turn(
    turn: number,
    prepared: PreparedCall,
    signal: AbortSignal | undefined,
  ): Promise<
    | { kind: "continue"; usage: Usage; prepared: PreparedCall }
    | {
        kind: "done" | "aborted" | "error";
        usage: Usage;
        error?: string;
        prepared: PreparedCall;
      }
  > {
    await this.dispatcher.emit("turn_start", { turn });
    const parentPath = this.parentSnapshot();
    const view = await this.viewProvider.view({
      sessionId: this.sessionId,
      turn,
      path: structuredClone(this.store.path(this.sessionId)) as StoredEntry[],
      model: this.model,
      prepared,
      head: this.injectedHead(),
      tools: this.tools,
      depth: this.parent?.depth ?? 0,
      ...(parentPath ? { parentPath } : {}),
    });
    assertPaired(view.messages);
    this.assertEpoch(view);
    await this.dispatcher.emit("view_compiled", {
      view: structuredClone(view),
    });
    const routed = this.applyRoute(view, prepared);
    if ("error" in routed) {
      await this.dispatcher.emit("turn_end", { turn, outcome: "error" });
      return {
        kind: "error",
        usage: emptyUsage(),
        error: routed.error,
        prepared,
      };
    }
    const active = routed.prepared;
    const sampling = mergeSampling(active.adapterDefaults, this.sampling);
    const request: Request = {
      model: this.model,
      head: view.head,
      messages: view.messages,
      cacheBreakpoints: [...view.cacheBreakpoints],
      sampling,
    };
    this.appendViewEpoch(view);
    this.appendHeader(view, sampling, active);
    this.store.append(this.sessionId, {
      kind: "manifest",
      payload: view.manifest,
      ...(view.details ? { details: view.details } : {}),
    });
    const streamed = await this.collectWithTurnRetry(
      turn,
      request,
      active,
      signal,
    );
    if (streamed === "aborted") {
      await this.dispatcher.emit("turn_end", { turn, outcome: "aborted" });
      return { kind: "aborted", usage: emptyUsage(), prepared: active };
    }
    const assistant = streamed;
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

    if (assistant.finish.kind === "aborted" || signal?.aborted) {
      await this.dispatcher.emit("turn_end", { turn, outcome: "aborted" });
      return { kind: "aborted", usage: assistant.usage, prepared: active };
    }
    if (assistant.finish.kind === "error") {
      await this.dispatcher.emit("turn_end", { turn, outcome: "error" });
      return {
        kind: "error",
        usage: assistant.usage,
        error: assistant.finish.error?.message ?? "model request failed",
        prepared: active,
      };
    }
    if (assistant.finish.kind === "tool_use") {
      const ran = await this.runTools(toolUses(assistant.content), signal);
      if (signal?.aborted) {
        await this.dispatcher.emit("turn_end", { turn, outcome: "aborted" });
        return { kind: "aborted", usage: assistant.usage, prepared: active };
      }
      if (ran.results.length > 0) {
        const payload: Message = { role: "user", content: ran.results };
        this.store.append(this.sessionId, {
          kind: "message",
          payload,
          ...(ran.toolResults.length > 0
            ? { details: { toolResults: ran.toolResults } }
            : {}),
        });
      }
      await this.dispatcher.emit("turn_end", { turn, outcome: "continue" });
      return { kind: "continue", usage: assistant.usage, prepared: active };
    }
    await this.dispatcher.emit("turn_end", { turn, outcome: "done" });
    return { kind: "done", usage: assistant.usage, prepared: active };
  }

  private async collectWithTurnRetry(
    turn: number,
    request: Request,
    active: PreparedCall,
    signal: AbortSignal | undefined,
  ): Promise<AssistantMessage | "aborted"> {
    let attempt = 0;
    while (true) {
      const assistant = await collect(
        this.adapter.stream(request, active, signal),
        { api: this.adapter.api },
      );
      if (assistant.finish.kind === "aborted" || signal?.aborted) {
        return assistant;
      }
      const reason = turnTransportReason(assistant.finish);
      const committed = assistantAfterLatestHeader(
        this.store.path(this.sessionId),
      );
      if (reason !== undefined && !committed && attempt < TURN_RETRY_LIMIT) {
        attempt += 1;
        await this.sleep(backoffMs(attempt, TURN_RETRY_BASE_MS), signal);
        if (signal?.aborted) return "aborted";
        this.store.append(this.sessionId, {
          kind: "turn_retry",
          payload: { turn, attempt, reason },
        });
        continue;
      }
      return assistant;
    }
  }

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
    ) as StoredEntry[];
  }

  private injectedHead(): Head {
    return {
      system: this.system,
      tools: this.tools.map((tool) => tool.def),
    };
  }

  private assertEpoch(view: CompiledView): void {
    const path = this.store.path(this.sessionId);
    const prev = lastHeader(path);
    const { latest, frozen } = epochState(path);
    const systemSame =
      prev !== undefined &&
      prev.systemPromptHash === systemHash(view.head.system);
    const toolsSame =
      prev !== undefined && prev.toolsHash === toolsHash(view.head.tools);
    if (prev && (!systemSame || !toolsSame) && view.epoch !== latest) {
      throw new HeadChangedMidEpochError();
    }
    assertHeadFrozen(view, prev, frozen);
  }

  private appendUser(text: string): void {
    const payload: Message = {
      role: "user",
      content: [{ type: "text", text }],
    };
    this.store.append(this.sessionId, { kind: "message", payload });
  }

  private appendViewEpoch(view: CompiledView): void {
    const latest = latestViewEpoch(this.store.path(this.sessionId));
    if (!view.epochStart && view.epoch <= latest) return;
    const epochHead = view.messages[0];
    this.store.append(this.sessionId, {
      kind: "view_epoch",
      payload: {
        epoch: view.epoch,
        reason: view.epochStart?.reason ?? "epoch-advanced",
      },
      ...(epochHead ? { details: { epochHead } } : {}),
    });
  }

  private appendHeader(
    view: CompiledView,
    sampling: Sampling,
    prepared: PreparedCall,
  ): void {
    const prev = lastHeader(this.store.path(this.sessionId));
    const systemPromptHash = systemHash(view.head.system);
    const toolsDigest = toolsHash(view.head.tools);
    const payload: RequestHeaderPayload = {
      provider: this.model.provider,
      model: this.model.model,
      systemPromptHash,
      toolsHash: toolsDigest,
      sampling: { ...sampling } as Record<string, unknown>,
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

  private async runTools(
    calls: ToolUseBlock[],
    signal: AbortSignal | undefined,
  ): Promise<{
    results: ToolResultBlock[];
    toolResults: {
      toolUseId: string;
      name: string;
      details: Record<string, unknown>;
    }[];
  }> {
    const results = [];
    const toolResults: {
      toolUseId: string;
      name: string;
      details: Record<string, unknown>;
    }[] = [];
    for (const call of calls) {
      if (signal?.aborted) break;
      const tool = this.tools.find((item) => item.def.name === call.name);
      const wrapped = tool
        ? {
            ...tool,
            execute: async (
              input: unknown,
              ctx: {
                sessionId: string;
                toolUseId: string;
                signal?: AbortSignal;
              },
            ) => {
              const raw = await tool.execute(input, ctx);
              if (raw.details) {
                toolResults.push({
                  toolUseId: call.id,
                  name: call.name,
                  details: raw.details,
                });
              }
              return raw;
            },
          }
        : undefined;
      results.push(
        await this.dispatcher.runTool(call, wrapped, {
          sessionId: this.sessionId,
          toolUseId: call.id,
          signal,
        }),
      );
    }
    return { results, toolResults };
  }

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
