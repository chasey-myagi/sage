/**
 * Subtask tools. A child is another compile of the same Log.
 *
 * Contract: `spawn_subtask` takes `{ query, budget?, readOnly?, model? }`.
 * `spawn_subtasks` takes `{ tasks }` of those objects and runs only when
 * every task is `readOnly: true`. The child session gets a new id. Its first
 * entry is `subtask`. The first user message is the fixed Begin line; the
 * query is the tail claim `subtask-query`, not that message. The child uses
 * the same system string and tool list as the caller, so the head stays
 * byte-identical. `readOnly` installs a guard that denies every tool whose
 * `isReadOnly` is false. Depth and fan-out are checked before a child is
 * created. `maxDepth` defaults to 2, `maxSubtasks` to 8, `maxConcurrent` to
 * 4. One child throwing does not cancel the others.
 *
 * Intentionally not in the Log: the semaphore, the per-session counters, and
 * the compiler closure. The `subtask` entry, the child's messages, and the
 * parent's tool_use / tool_result are the records. Child usage stays on the
 * tool result details and is not added to the parent run.
 */

import { randomUUID } from "node:crypto";
import type { HarnessTool, LogStore, ViewProvider } from "@sage/core";
import { AgentSession } from "@sage/core";
import type { ModelRef, ProviderAdapter, Usage } from "@sage/provider/contract";

const BEGIN = 'Begin the subtask described in <claim id="subtask-query">.';
const DEFAULT_BUDGET = 8000;

const EMPTY_USAGE: Usage = {
  inputOther: 0,
  inputCacheRead: 0,
  inputCacheCreation: 0,
  output: 0,
};

type TaskInput = {
  query: string;
  budget?: number;
  readOnly?: boolean;
  model?: ModelRef;
};

type TaskResult = {
  content: string;
  isError: boolean;
  details: {
    sessionId: string;
    outcome: string;
    usage: Usage;
    chunks: string[];
  };
};

export function subtaskTools(opts: {
  store: LogStore;
  adapters: Record<string, ProviderAdapter>;
  makeCompiler: (o: { budget: number; query: string }) => ViewProvider;
  tools: HarnessTool[];
  system: string;
  maxDepth?: number;
  maxSubtasks?: number;
  maxConcurrent?: number;
}): { spawn: HarnessTool; spawnMany: HarnessTool } {
  const maxDepth = opts.maxDepth ?? 2;
  const maxSubtasks = opts.maxSubtasks ?? 8;
  const maxConcurrent = Math.max(1, opts.maxConcurrent ?? 4);
  const spawned = new Map<string, number>();

  const taskSchema = {
    type: "object",
    properties: {
      query: {
        type: "string",
        description:
          "Question for the child compile. It is appended after the shared prefix.",
      },
      budget: {
        type: "number",
        description: "Token budget for the child compile.",
      },
      readOnly: {
        type: "boolean",
        description:
          "When true, tools with isReadOnly false are denied. The tool list itself is unchanged.",
      },
      model: {
        type: "object",
        properties: {
          provider: { type: "string" },
          model: { type: "string" },
        },
        required: ["provider", "model"],
      },
    },
    required: ["query"],
  };

  async function runTask(
    parentSessionId: string,
    task: TaskInput,
    signal: AbortSignal | undefined,
  ): Promise<TaskResult> {
    const budget = task.budget ?? DEFAULT_BUDGET;
    const readOnly = task.readOnly === true;
    const model = task.model ?? modelFromLog(opts.store, parentSessionId);
    if (!model) {
      return failed("", "error", "Parent model is unknown.");
    }
    const leaf = opts.store.leaf(parentSessionId);
    if (!leaf) return failed("", "error", "Parent session is empty.");
    const sessionId = randomUUID();
    let session: AgentSession;
    try {
      session = new AgentSession({
        store: opts.store,
        sessionId,
        adapters: opts.adapters,
        model,
        system: opts.system,
        tools: opts.tools,
        viewProvider: opts.makeCompiler({ budget, query: task.query }),
        parent: {
          sessionId: parentSessionId,
          leafId: leaf,
          depth: depthOf(opts.store, parentSessionId) + 1,
        },
      });
    } catch (error) {
      return failed(sessionId, "error", errorText(error));
    }
    opts.store.append(sessionId, {
      kind: "subtask",
      payload: {
        parentSessionId,
        parentLeaf: leaf,
        query: task.query,
        budget,
        readOnly,
      },
    });
    if (readOnly) {
      session.hooks.guard((call) => {
        const tool = opts.tools.find((item) => item.def.name === call.name);
        if (tool && !tool.isReadOnly) return "read-only subtask";
        return undefined;
      });
    }
    try {
      const summary = await session.run(BEGIN, { signal });
      const chunks = chunkRefs(opts.store, sessionId);
      return {
        content: formatResult(
          sessionId,
          summary.outcome,
          lastAssistantText(opts.store, sessionId),
          chunks,
        ),
        isError: summary.outcome !== "done",
        details: {
          sessionId,
          outcome: summary.outcome,
          usage: summary.usage,
          chunks,
        },
      };
    } catch (error) {
      return failed(sessionId, "error", errorText(error));
    }
  }

  function depthError(): TaskResult {
    return failed(
      "",
      "error",
      `Subtask depth limit reached (maxDepth=${maxDepth})`,
    );
  }

  function capError(): TaskResult {
    return failed(
      "",
      "error",
      `Subtask limit reached (maxSubtasks=${maxSubtasks})`,
    );
  }

  const spawn: HarnessTool = {
    def: {
      name: "spawn_subtask",
      description:
        "Recompile this session on one question and budget. The child shares the head and frozen prefix. readOnly denies tools that are not read-only without changing the tool list.",
      inputSchema: taskSchema,
    },
    isReadOnly: false,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute(input, ctx) {
      const task = readTask(input);
      if (!task || task.query.trim().length === 0) {
        return {
          content: "spawn_subtask: query is required",
          isError: true,
        };
      }
      if (depthOf(opts.store, ctx.sessionId) >= maxDepth) {
        return depthError();
      }
      if (!reserve(spawned, ctx.sessionId, 1, maxSubtasks)) return capError();
      return runTask(ctx.sessionId, task, ctx.signal);
    },
  };

  const spawnMany: HarnessTool = {
    def: {
      name: "spawn_subtasks",
      description:
        "Recompile several read-only questions in parallel. The pool rejects the call unless every task sets readOnly to true.",
      inputSchema: {
        type: "object",
        properties: {
          tasks: { type: "array", items: taskSchema },
        },
        required: ["tasks"],
      },
    },
    isReadOnly: false,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute(input, ctx) {
      const tasks = readTasks(input);
      if (!tasks || tasks.length === 0) {
        return { content: "spawn_subtasks: tasks are required", isError: true };
      }
      if (tasks.some((task) => task.query.trim().length === 0)) {
        return { content: "spawn_subtasks: query is required", isError: true };
      }
      if (tasks.some((task) => task.readOnly !== true)) {
        return {
          content: "Parallel pool only accepts read-only subtasks.",
          isError: true,
        };
      }
      if (depthOf(opts.store, ctx.sessionId) >= maxDepth) return depthError();
      if (!reserve(spawned, ctx.sessionId, tasks.length, maxSubtasks)) {
        return capError();
      }
      const results = await runPool(tasks, maxConcurrent, (task) =>
        runTask(ctx.sessionId, task, ctx.signal),
      );
      return {
        content: results.map((result) => result.content).join("\n\n"),
        isError: results.some((result) => result.isError),
        details: {
          results: results.map((result) => result.details),
        },
      };
    },
  };

  return { spawn, spawnMany };
}

function reserve(
  spawned: Map<string, number>,
  sessionId: string,
  count: number,
  max: number,
): boolean {
  const used = spawned.get(sessionId) ?? 0;
  if (used + count > max) return false;
  spawned.set(sessionId, used + count);
  return true;
}

async function runPool<T>(
  tasks: readonly T[],
  limit: number,
  run: (task: T) => Promise<TaskResult>,
): Promise<TaskResult[]> {
  const results = new Array<TaskResult>(tasks.length);
  let cursor = 0;
  const workers = Math.max(1, Math.min(limit, tasks.length));
  await Promise.all(
    Array.from({ length: workers }, async () => {
      while (true) {
        const index = cursor;
        cursor += 1;
        if (index >= tasks.length) return;
        const task = tasks[index];
        if (task === undefined) return;
        try {
          results[index] = await run(task);
        } catch (error) {
          results[index] = failed("", "error", errorText(error));
        }
      }
    }),
  );
  return results;
}

function failed(
  sessionId: string,
  outcome: string,
  message: string,
): TaskResult {
  return {
    content: sessionId
      ? formatResult(sessionId, outcome, message, [])
      : message,
    isError: true,
    details: {
      sessionId,
      outcome,
      usage: { ...EMPTY_USAGE },
      chunks: [],
    },
  };
}

function formatResult(
  sessionId: string,
  outcome: string,
  text: string,
  chunks: readonly string[],
): string {
  return `Subtask ${sessionId} ${outcome}. ${text}\nChunks: ${chunks.join(", ")}`;
}

function readTasks(input: unknown): TaskInput[] | undefined {
  if (!isRecord(input) || !Array.isArray(input.tasks)) return undefined;
  const tasks: TaskInput[] = [];
  for (const value of input.tasks) {
    const task = readTask(value);
    if (!task) return undefined;
    tasks.push(task);
  }
  return tasks;
}

function readTask(input: unknown): TaskInput | undefined {
  if (!isRecord(input) || typeof input.query !== "string") return undefined;
  const task: TaskInput = { query: input.query };
  if (typeof input.budget === "number" && Number.isFinite(input.budget)) {
    task.budget = input.budget;
  }
  if (typeof input.readOnly === "boolean") task.readOnly = input.readOnly;
  const model = readModel(input.model);
  if (model) task.model = model;
  return task;
}

function readModel(value: unknown): ModelRef | undefined {
  if (!isRecord(value)) return undefined;
  if (typeof value.provider !== "string" || typeof value.model !== "string") {
    return undefined;
  }
  return { provider: value.provider, model: value.model };
}

function modelFromLog(
  store: LogStore,
  sessionId: string,
): ModelRef | undefined {
  const path = store.path(sessionId);
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "request_header" || !isRecord(entry.payload)) continue;
    const { provider, model } = entry.payload;
    if (typeof provider === "string" && typeof model === "string") {
      return { provider, model };
    }
  }
  return undefined;
}

function depthOf(
  store: LogStore,
  sessionId: string,
  seen = new Set<string>(),
): number {
  if (seen.has(sessionId)) return 0;
  seen.add(sessionId);
  const entry = store.path(sessionId).find((item) => item.kind === "subtask");
  if (!entry || !isRecord(entry.payload)) return 0;
  const parent = entry.payload.parentSessionId;
  if (typeof parent !== "string") return 0;
  return depthOf(store, parent, seen) + 1;
}

function chunkRefs(store: LogStore, sessionId: string): string[] {
  const path = store.path(sessionId);
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "manifest" || !isRecord(entry.payload)) continue;
    if (!Array.isArray(entry.payload.chunks)) return [];
    return entry.payload.chunks.flatMap((chunk) => {
      if (!isRecord(chunk) || typeof chunk.id !== "string") return [];
      return [`child:${sessionId}#${chunk.id}`];
    });
  }
  return [];
}

function lastAssistantText(store: LogStore, sessionId: string): string {
  const path = store.path(sessionId);
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "message" || !isRecord(entry.payload)) continue;
    if (entry.payload.role !== "assistant") continue;
    if (!Array.isArray(entry.payload.content)) continue;
    let text = "";
    for (const block of entry.payload.content) {
      if (!isRecord(block) || block.type !== "text") continue;
      if (typeof block.text === "string") text += block.text;
    }
    if (text.length > 0) return text;
  }
  return "";
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
