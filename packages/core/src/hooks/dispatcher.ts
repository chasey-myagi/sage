/**
 * Hook dispatcher.
 *
 * Contract: observers run in registration order. A throw is recorded and the
 * next observer still runs. The tool pipeline is fixed: tool_pre, then guard,
 * then around(execute), then tool_post, then tool_result. A pre deny is final.
 * A guard may only add a deny. Neither denial executes the tool.
 *
 * Intentionally not in the Log: the callback lists. This object never sees
 * the store.
 */

import type { ToolResultBlock, ToolUseBlock } from "@sage/provider/contract";
import type { HarnessTool } from "../tools.js";
import type { Decision, HookEvents, Hooks, ObserverError } from "./types.js";

type Observer = (payload: never) => void | Promise<void>;
type PreFn = (payload: HookEvents["tool_pre"]) => Decision | Promise<Decision>;
type PostFn = (
  payload: HookEvents["tool_post"],
) => ToolResultBlock | undefined | Promise<ToolResultBlock | undefined>;
type GuardFn = (call: ToolUseBlock) => string | undefined;
type AroundFn = (
  call: ToolUseBlock,
  next: () => Promise<ToolResultBlock>,
) => Promise<ToolResultBlock>;

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function denied(id: string, reason: string): ToolResultBlock {
  return {
    type: "tool_result",
    toolUseId: id,
    content: `Denied: ${reason}`,
    isError: true,
  };
}

export class HookDispatcher implements Hooks {
  private readonly observers = new Map<string, Observer[]>();
  private readonly pres: PreFn[] = [];
  private readonly posts: PostFn[] = [];
  private readonly guards: GuardFn[] = [];
  private readonly arounds: AroundFn[] = [];

  constructor(private readonly errors: ObserverError[]) {}

  observe<E extends keyof HookEvents>(
    event: E,
    fn: (payload: Readonly<HookEvents[E]>) => void | Promise<void>,
  ): () => void {
    const list = this.observers.get(event) ?? [];
    const observer = fn as Observer;
    list.push(observer);
    this.observers.set(event, list);
    return () => {
      const current = this.observers.get(event);
      if (!current) return;
      const index = current.indexOf(observer);
      if (index >= 0) current.splice(index, 1);
    };
  }

  on(
    event: "tool_pre",
    fn: (payload: HookEvents["tool_pre"]) => Decision | Promise<Decision>,
  ): () => void;
  on(
    event: "tool_post",
    fn: (
      payload: HookEvents["tool_post"],
    ) => ToolResultBlock | undefined | Promise<ToolResultBlock | undefined>,
  ): () => void;
  on(event: "tool_pre" | "tool_post", fn: PreFn | PostFn): () => void {
    if (event === "tool_pre") {
      const pre = fn as PreFn;
      this.pres.push(pre);
      return () => {
        const index = this.pres.indexOf(pre);
        if (index >= 0) this.pres.splice(index, 1);
      };
    }
    const post = fn as PostFn;
    this.posts.push(post);
    return () => {
      const index = this.posts.indexOf(post);
      if (index >= 0) this.posts.splice(index, 1);
    };
  }

  guard(fn: GuardFn): () => void {
    this.guards.push(fn);
    return () => {
      const index = this.guards.indexOf(fn);
      if (index >= 0) this.guards.splice(index, 1);
    };
  }

  around(fn: AroundFn): () => void {
    this.arounds.push(fn);
    return () => {
      const index = this.arounds.indexOf(fn);
      if (index >= 0) this.arounds.splice(index, 1);
    };
  }

  async emit<E extends keyof HookEvents>(
    event: E,
    payload: HookEvents[E],
  ): Promise<void> {
    const list = this.observers.get(event) ?? [];
    for (const fn of list) {
      try {
        await fn(payload as never);
      } catch (error) {
        this.errors.push({ event, error: errorText(error) });
      }
    }
  }

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
        return {
          type: "tool_result",
          toolUseId: call.id,
          content: errorText(error),
          isError: true,
        };
      }
    };

    let wrapped = execute;
    for (let i = this.arounds.length - 1; i >= 0; i -= 1) {
      const around = this.arounds[i];
      if (!around) continue;
      const inner = wrapped;
      wrapped = () => around(call, inner);
    }
    return this.finishTool(call, await wrapped());
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
