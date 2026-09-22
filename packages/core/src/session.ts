/**
 * Minimal agent loop.
 *
 * One turn, in order:
 * turn_start → prepared = adapter.prepare(model) once per run →
 * view = await viewProvider.view(ctx) → assertPaired → assertHeadFrozen →
 * structuredClone(view) is what observe("view_compiled") receives →
 * append request_header (system and tools text only when that hash differs
 * from the previous request_header, or there is none) → adapter.stream →
 * collect → append message(assistant) with details
 * {responseModel, responseId, api, usage, finish} → llm_end →
 * if finish.kind is tool_use, run the tool pipeline for each tool_use and
 * append one user message of results, then turn_end continue;
 * otherwise turn_end done.
 * abort, an adapter finish error, or a throw appends terminal and stops.
 * Using up maxTurns appends terminal {outcome:"max_turns"}.
 *
 * Contract: the kernel does not compress. Hooks never receive the store.
 * ViewContext.path is a structured clone taken before view().
 *
 * Intentionally not in the Log: the hook list, the abort signal, observer
 * errors, and tool execute details. Model-visible text is the message and
 * request_header entries.
 */

import type {
  ContentBlock,
  Head,
  Message,
  ModelRef,
  PreparedCall,
  ProviderAdapter,
  Request,
  Sampling,
  ToolUseBlock,
  Usage,
} from "@sage/provider/contract";
import { HookDispatcher } from "./hooks/dispatcher.js";
import type { Hooks } from "./hooks/types.js";
import type { RequestHeaderPayload } from "./log/kinds.js";
import type { LogStore } from "./log/store.js";
import type { StoredEntry } from "./log/types.js";
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
  observerErrors: { event: string; error: string }[];
  error?: string;
};

const EMPTY_USAGE: Usage = {
  inputOther: 0,
  inputCacheRead: 0,
  inputCacheCreation: 0,
  output: 0,
};

function addUsage(left: Usage, right: Usage): Usage {
  return {
    inputOther: left.inputOther + right.inputOther,
    inputCacheRead: left.inputCacheRead + right.inputCacheRead,
    inputCacheCreation: left.inputCacheCreation + right.inputCacheCreation,
    output: left.output + right.output,
  };
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
  if (
    typeof payload !== "object" ||
    payload === null ||
    Array.isArray(payload)
  ) {
    return undefined;
  }
  return payload as RequestHeaderPayload;
}

function epochState(path: readonly StoredEntry[]): {
  latest: number;
  frozen: number;
} {
  let latest = 0;
  let frozen = 0;
  let sawHeader = false;
  for (const entry of path) {
    if (entry.kind === "view_epoch") {
      const epoch = (entry.payload as { epoch?: unknown }).epoch;
      if (typeof epoch === "number") latest = epoch;
      continue;
    }
    if (entry.kind === "request_header") {
      frozen = latest;
      sawHeader = true;
    }
  }
  if (!sawHeader) frozen = latest;
  return { latest, frozen };
}

function lastHeader(
  path: readonly StoredEntry[],
): RequestHeaderPayload | undefined {
  for (let i = path.length - 1; i >= 0; i -= 1) {
    const entry = path[i];
    if (entry?.kind === "request_header") return asHeader(entry.payload);
  }
  return undefined;
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
  private readonly adapter: ProviderAdapter;
  private readonly model: ModelRef;
  private readonly system: string;
  private readonly tools: HarnessTool[];
  private readonly viewProvider: ViewProvider;
  private readonly sampling: Sampling | undefined;

  constructor(opts: {
    store: LogStore;
    sessionId: string;
    adapter: ProviderAdapter;
    model: ModelRef;
    system: string;
    tools: HarnessTool[];
    viewProvider?: ViewProvider;
    sampling?: Sampling;
  }) {
    this.store = opts.store;
    this.sessionId = opts.sessionId;
    this.adapter = opts.adapter;
    this.model = opts.model;
    this.system = opts.system;
    this.tools = opts.tools;
    this.viewProvider = opts.viewProvider ?? passthroughView;
    this.sampling = opts.sampling;
    this.dispatcher = new HookDispatcher(this.observerErrors);
    this.hooks = this.dispatcher;
  }

  async run(
    userText: string,
    opts?: { signal?: AbortSignal; maxTurns?: number },
  ): Promise<RunSummary> {
    const signal = opts?.signal;
    const maxTurns = opts?.maxTurns ?? 50;
    this.observerErrors.length = 0;
    const prepared = this.adapter.prepare(this.model);
    await this.dispatcher.emit("session_start", { sessionId: this.sessionId });
    this.appendUser(userText);

    let turns = 0;
    let usage = { ...EMPTY_USAGE };
    try {
      while (turns < maxTurns) {
        if (signal?.aborted) {
          return await this.finish("aborted", turns, usage);
        }
        turns += 1;
        const outcome = await this.turn(turns, prepared, signal);
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
    | { kind: "continue"; usage: Usage }
    | { kind: "done" | "aborted" | "error"; usage: Usage; error?: string }
  > {
    await this.dispatcher.emit("turn_start", { turn });
    const view = await this.viewProvider.view({
      sessionId: this.sessionId,
      turn,
      path: structuredClone(this.store.path(this.sessionId)) as StoredEntry[],
      model: this.model,
      prepared,
      head: this.injectedHead(),
    });
    assertPaired(view.messages);
    this.assertEpoch(view);
    const sampling = mergeSampling(prepared.adapterDefaults, this.sampling);
    const request: Request = {
      model: this.model,
      head: view.head,
      messages: view.messages,
      cacheBreakpoints: [...view.cacheBreakpoints],
      sampling,
    };
    await this.dispatcher.emit("view_compiled", {
      view: structuredClone(view),
    });
    this.appendHeader(view, sampling, prepared);
    // Dynamic so dist/index.js can load under plain Node for the log manifest.
    // The contract barrel is TypeScript source; a static import breaks that load.
    const { collect } = await import("@sage/provider/contract");
    const assistant = await collect(
      this.adapter.stream(request, prepared, signal),
      {
        responseModel: this.model.model,
        responseId: "",
        api: this.adapter.api,
      },
    );
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
      return { kind: "aborted", usage: assistant.usage };
    }
    if (assistant.finish.kind === "error") {
      await this.dispatcher.emit("turn_end", { turn, outcome: "error" });
      return {
        kind: "error",
        usage: assistant.usage,
        error: assistant.finish.error?.message ?? "model request failed",
      };
    }
    if (assistant.finish.kind === "tool_use") {
      const results = await this.runTools(toolUses(assistant.content), signal);
      if (signal?.aborted) {
        await this.dispatcher.emit("turn_end", { turn, outcome: "aborted" });
        return { kind: "aborted", usage: assistant.usage };
      }
      if (results.length > 0) {
        const payload: Message = { role: "user", content: results };
        this.store.append(this.sessionId, { kind: "message", payload });
      }
      await this.dispatcher.emit("turn_end", { turn, outcome: "continue" });
      return { kind: "continue", usage: assistant.usage };
    }
    await this.dispatcher.emit("turn_end", { turn, outcome: "done" });
    return { kind: "done", usage: assistant.usage };
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
  ) {
    const results = [];
    for (const call of calls) {
      if (signal?.aborted) break;
      const tool = this.tools.find((item) => item.def.name === call.name);
      results.push(
        await this.dispatcher.runTool(call, tool, {
          sessionId: this.sessionId,
          signal,
        }),
      );
    }
    return results;
  }

  private async finish(
    outcome: RunSummary["outcome"],
    turns: number,
    usage: Usage,
    error?: string,
  ): Promise<RunSummary> {
    const summary: RunSummary = {
      outcome,
      turns,
      usage,
      observerErrors: this.observerErrors,
    };
    if (error !== undefined) summary.error = error;
    const payload: { outcome: string; usage: Usage; error?: string } = {
      outcome,
      usage,
    };
    if (error !== undefined) payload.error = error;
    this.store.append(this.sessionId, { kind: "terminal", payload });
    await this.dispatcher.emit("session_end", { summary });
    return summary;
  }
}
