/**
 * Hook dispatcher.
 *
 * Guarantees: observers run in registration order. A throw is recorded and the
 * next observer still runs. The tool pipeline is fixed: tool_pre, then guard,
 * then around(execute), then tool_post, then tool_result. A pre deny is final.
 * A guard may only add a deny. Neither denial executes the tool. A throw from
 * execute, synchronous or rejected, or from around, becomes a tool_result
 * `{ isError: true, content: "<name>: <message>" }` and does not escape
 * runTool. A known tool whose input is not a JSON object does not execute.
 *
 * Intentionally not in the Log: the callback lists. This object never sees
 * the store.
 */

import {
  errorText,
  isRecord,
  type ToolResultBlock,
  type ToolUseBlock,
} from "@sage/provider/contract";
import type { HarnessTool } from "../tools.js";
import type { Decision, HookEvents, Hooks, ObserverError } from "./types.js";

type ObserverFn<E extends keyof HookEvents> = (
  payload: Readonly<HookEvents[E]>,
) => void | Promise<void>;
type ObserverLists = { [E in keyof HookEvents]: ObserverFn<E>[] };
type PreFn = (payload: HookEvents["tool_pre"]) => Decision | Promise<Decision>;
type PostFn = (
  payload: HookEvents["tool_post"],
) => ToolResultBlock | undefined | Promise<ToolResultBlock | undefined>;
type GuardFn = (call: ToolUseBlock) => string | undefined;
type AroundFn = (
  call: ToolUseBlock,
  next: () => Promise<ToolResultBlock>,
) => Promise<ToolResultBlock>;

/** Appends `item` to `list`; the returned function removes that one entry. */
function register<T>(list: T[], item: T): () => void {
  list.push(item);
  return () => {
    const index = list.indexOf(item);
    if (index >= 0) list.splice(index, 1);
  };
}

function denied(id: string, reason: string): ToolResultBlock {
  return {
    type: "tool_result",
    toolUseId: id,
    content: `Denied: ${reason}`,
    isError: true,
  };
}

/** Characters of unusable argument text quoted back to the model. */
const ARGUMENTS_PREVIEW_CHARS = 500;

/**
 * The error for a call whose arguments are not a JSON object. It quotes
 * the streamed text because the adapter replays the call with `{}`, so the
 * model would otherwise never see what it sent.
 */
function unusableArguments(call: ToolUseBlock): ToolResultBlock {
  const preview =
    call.rawInput.length > ARGUMENTS_PREVIEW_CHARS
      ? `${call.rawInput.slice(0, ARGUMENTS_PREVIEW_CHARS)}...`
      : call.rawInput;
  return {
    type: "tool_result",
    toolUseId: call.id,
    content: `${call.name}: arguments are not a JSON object, so the tool did not run. Resend the call with arguments that match its input schema. Received: ${preview}`,
    isError: true,
  };
}

function toolFailure(call: ToolUseBlock, error: unknown): ToolResultBlock {
  return {
    type: "tool_result",
    toolUseId: call.id,
    content: `${call.name}: ${errorText(error)}`,
    isError: true,
  };
}

/**
 * The session's `Hooks` implementation. `errors` is the caller's array;
 * observer throws are pushed onto it.
 */
export class HookDispatcher implements Hooks {
  // One list per event, created up front: TypeScript can read a mapped type
  // through a generic key but cannot assign through one.
  private readonly observers: ObserverLists = {
    session_start: [],
    turn_start: [],
    view_compiled: [],
    llm_end: [],
    tool_pre: [],
    tool_post: [],
    tool_result: [],
    turn_end: [],
    session_end: [],
  };
  private readonly pres: PreFn[] = [];
  private readonly posts: PostFn[] = [];
  private readonly guards: GuardFn[] = [];
  private readonly arounds: AroundFn[] = [];

  constructor(private readonly errors: ObserverError[]) {}

  observe<E extends keyof HookEvents>(event: E, fn: ObserverFn<E>): () => void {
    return register(this.observers[event], fn);
  }

  on(event: "tool_pre", fn: PreFn): () => void;
  on(event: "tool_post", fn: PostFn): () => void;
  on(...args: ["tool_pre", PreFn] | ["tool_post", PostFn]): () => void {
    const [event, fn] = args;
    if (event === "tool_pre") return register(this.pres, fn);
    return register(this.posts, fn);
  }

  guard(fn: GuardFn): () => void {
    return register(this.guards, fn);
  }

  around(fn: AroundFn): () => void {
    return register(this.arounds, fn);
  }

  /**
   * Runs `event`'s observers in registration order, awaiting each. A throw
   * or rejection is recorded on `errors` and the next observer still runs;
   * `emit` itself never throws.
   */
  async emit<E extends keyof HookEvents>(
    event: E,
    payload: HookEvents[E],
  ): Promise<void> {
    for (const fn of this.observers[event]) {
      try {
        await fn(payload);
      } catch (error) {
        this.errors.push({ event, error: errorText(error) });
      }
    }
  }

  /**
   * Runs one call through the fixed pipeline and returns its tool_result;
   * never throws for a tool failure. An unknown `tool` yields an error
   * result after the pre hooks and guards ran, and so does an input that is
   * not a JSON object (after any pre-hook `updatedInput`); neither executes.
   * `tool_post` hooks may replace the result; observers then see the final
   * one as `tool_result`.
   */
  async runTool(
    call: ToolUseBlock,
    tool: HarnessTool | undefined,
    exec: { sessionId: string; toolUseId: string; signal?: AbortSignal },
  ): Promise<ToolResultBlock> {
    let input = call.input;
    for (const pre of this.pres) {
      const decision = await pre({ call });
      if (!decision) continue;
      if (decision.updatedInput !== undefined) input = decision.updatedInput;
      if (decision.deny !== undefined) {
        return this.finishTool(call, denied(call.id, decision.deny));
      }
    }
    for (const guard of this.guards) {
      const reason = guard(call);
      if (typeof reason === "string") {
        return this.finishTool(call, denied(call.id, reason));
      }
    }
    if (!tool) {
      return this.finishTool(call, {
        type: "tool_result",
        toolUseId: call.id,
        content: `Unknown tool: ${call.name}`,
        isError: true,
      });
    }
    if (!isRecord(input)) return this.finishTool(call, unusableArguments(call));

    const execute = async (): Promise<ToolResultBlock> => {
      try {
        const raw = await tool.execute(input, exec);
        return {
          type: "tool_result",
          toolUseId: call.id,
          content: raw.content,
          isError: raw.isError,
        };
      } catch (error) {
        return toolFailure(call, error);
      }
    };

    let wrapped = execute;
    for (let i = this.arounds.length - 1; i >= 0; i -= 1) {
      const around = this.arounds[i];
      if (!around) continue;
      const inner = wrapped;
      wrapped = () => around(call, inner);
    }
    let result: ToolResultBlock;
    try {
      result = await wrapped();
    } catch (error) {
      result = toolFailure(call, error);
    }
    return this.finishTool(call, result);
  }

  private async finishTool(
    call: ToolUseBlock,
    result: ToolResultBlock,
  ): Promise<ToolResultBlock> {
    let current = result;
    for (const post of this.posts) {
      const next = await post({ call, result: current });
      if (next) current = next;
    }
    await this.emit("tool_result", { call, result: current });
    return current;
  }
}
