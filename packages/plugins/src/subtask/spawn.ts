/**
 * Subtask tools. A child is another compile of the same Log: a new session
 * whose view starts with the caller's frozen request.
 *
 * Contract: the child uses the caller's system string and tool list, so
 * its head is byte-identical and hits the same prefix cache. Depth and
 * fan-out are checked before any child is created. A child that cannot
 * start, or whose run throws, becomes an `isError` tool result. Task input
 * and presets are in `task-input.ts`; batch admission and the pool are in
 * `admission.ts`.
 *
 * Intentionally not in the Log: the pool, the per-session counters, and
 * the compiler closure. The child's `subtask` entry, its messages, and the
 * parent's tool_use / tool_result are the records. Both tools report their
 * children as `details.childRuns` (one item per child); the kernel copies
 * them onto the parent user message, does not add them to the parent run's
 * usage, and `runHealth` sums them under `subtasks`.
 */

import { randomUUID } from "node:crypto";
import type { ChildRun, HarnessTool, LogStore, ViewProvider } from "@sage/core";
import {
  AgentSession,
  errorText,
  isRecord,
  lastRequestHeader,
} from "@sage/core";
import {
  emptyUsage,
  type ModelRef,
  type ProviderAdapter,
  type Usage,
} from "@sage/provider/contract";
import { admissionError, reserve, runPool } from "./admission.js";
import type { PresetFs } from "./presets.js";
import {
  applyPreset,
  presetsOf,
  readTask,
  readTasks,
  TASK_SCHEMA,
  type TaskInput,
} from "./task-input.js";

/**
 * Default child compile budget in tokens, used when neither the task nor
 * its preset sets one.
 */
export const DEFAULT_SUBTASK_BUDGET = 8000;
/** Default `maxDepth`: how many levels of nested subtasks may exist. */
export const DEFAULT_MAX_DEPTH = 2;
/** Default `maxSubtasks`: children one session may spawn in total. */
export const DEFAULT_MAX_SUBTASKS = 8;
/** Default `maxConcurrent`: children of one call that run at once. */
export const DEFAULT_MAX_CONCURRENT = 4;

/**
 * The child's first user message. The question itself is the tail claim
 * `subtask-query`, not this message.
 */
const BEGIN = 'Begin the subtask described in <claim id="subtask-query">.';

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

/** Options of `subtaskTools`. */
export type SubtaskToolsOptions = {
  store: LogStore;
  adapters: Record<string, ProviderAdapter>;
  /** Builds the child's view provider for one budget and question. */
  makeCompiler: (o: { budget: number; query: string }) => ViewProvider;
  /** The caller's tool list; the child gets the same list. */
  tools: HarnessTool[];
  /** The caller's system string; the child gets the same string. */
  system: string;
  /** Defaults to `DEFAULT_MAX_DEPTH`. */
  maxDepth?: number;
  /** Defaults to `DEFAULT_MAX_SUBTASKS`. */
  maxSubtasks?: number;
  /** Defaults to `DEFAULT_MAX_CONCURRENT`; values below 1 become 1. */
  maxConcurrent?: number;
  /** Project root for `.sage/subtasks` presets; no presets without it. */
  cwd?: string;
  /** User home for `.sage/subtasks` presets. */
  home?: string;
  /** Test seam. Production reads presets with `node:fs`. */
  fs?: PresetFs;
};

/**
 * Creates `spawn_subtask` (one task) and `spawn_subtasks` (a parallel
 * batch). Both tools share one spawn tally per calling session.
 */
export function subtaskTools(opts: SubtaskToolsOptions): {
  spawn: HarnessTool;
  spawnMany: HarnessTool;
} {
  const maxDepth = opts.maxDepth ?? DEFAULT_MAX_DEPTH;
  const maxSubtasks = opts.maxSubtasks ?? DEFAULT_MAX_SUBTASKS;
  const spawned = new Map<string, number>();
  const deps: SpawnDeps = {
    opts,
    maxConcurrent: Math.max(1, opts.maxConcurrent ?? DEFAULT_MAX_CONCURRENT),
    admit(sessionId, count) {
      if (depthOf(opts.store, sessionId) >= maxDepth) {
        return {
          content: `Subtask depth limit reached (maxDepth=${maxDepth})`,
          isError: true,
        };
      }
      if (!reserve(spawned, sessionId, count, maxSubtasks)) {
        return {
          content: `Subtask limit reached (maxSubtasks=${maxSubtasks})`,
          isError: true,
        };
      }
      return undefined;
    },
  };
  return { spawn: spawnTool(deps), spawnMany: spawnManyTool(deps) };
}

type SpawnDeps = {
  opts: SubtaskToolsOptions;
  maxConcurrent: number;
  /**
   * Checks depth, then reserves `count` spawns for the session. Returns
   * the refusal as a tool result without details (no child ran), or
   * undefined when admitted.
   */
  admit(
    sessionId: string,
    count: number,
  ): { content: string; isError: true } | undefined;
};

function spawnTool(deps: SpawnDeps): HarnessTool {
  return {
    def: {
      name: "spawn_subtask",
      description:
        "Recompile this session on one question and budget. Pass query, or a preset name plus args. Explicit fields override the preset. The child shares the head and frozen prefix. readOnly denies tools that are not read-only without changing the tool list.",
      inputSchema: TASK_SCHEMA,
    },
    isReadOnly: false,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute(input, ctx) {
      const raw = readTask(input);
      if (!raw) {
        return {
          content: "spawn_subtask: query is required",
          isError: true,
        };
      }
      const task = applyPreset(raw, presetsOf(deps.opts));
      if (typeof task === "string") {
        return { content: `spawn_subtask: ${task}`, isError: true };
      }
      const refused = deps.admit(ctx.sessionId, 1);
      if (refused) return refused;
      const result = await runTask(deps.opts, ctx.sessionId, task, ctx.signal);
      return {
        content: result.content,
        isError: result.isError,
        details: { childRuns: [result.details] satisfies ChildRun[] },
      };
    },
  };
}

/**
 * `spawn_subtasks` takes `{ tasks }`. The whole call is rejected when any
 * task is invalid or the batch has an access conflict; otherwise the
 * results are joined in task order.
 */
function spawnManyTool(deps: SpawnDeps): HarnessTool {
  return {
    def: {
      name: "spawn_subtasks",
      description:
        "Recompile several questions in parallel. The pool accepts the call when every task is read-only, or when no pair shares a write key. A conflict rejects the whole call and names the pair.",
      inputSchema: {
        type: "object",
        properties: {
          tasks: { type: "array", items: TASK_SCHEMA },
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
      const tasks = resolveBatch(input, deps.opts);
      if (typeof tasks === "string") return { content: tasks, isError: true };
      const refused = deps.admit(ctx.sessionId, tasks.length);
      if (refused) return refused;
      const results = await runPool(
        tasks,
        deps.maxConcurrent,
        (task) => runTask(deps.opts, ctx.sessionId, task, ctx.signal),
        (error) => failed("", "error", errorText(error)),
      );
      return {
        content: results.map((result) => result.content).join("\n\n"),
        isError: results.some((result) => result.isError),
        details: {
          childRuns: results.map(
            (result) => result.details,
          ) satisfies ChildRun[],
        },
      };
    },
  };
}

/**
 * Parses and admits a `spawn_subtasks` batch: every task valid, every
 * preset resolved, every query non-blank, and no access conflict. Returns
 * the tool_result text of the first failure.
 */
function resolveBatch(
  input: unknown,
  opts: Pick<SubtaskToolsOptions, "cwd" | "home" | "fs">,
): TaskInput[] | string {
  const rawTasks = readTasks(input);
  if (!rawTasks || rawTasks.length === 0) {
    return "spawn_subtasks: tasks are required";
  }
  const tasks: TaskInput[] = [];
  for (const raw of rawTasks) {
    const task = applyPreset(raw, presetsOf(opts));
    if (typeof task === "string") return `spawn_subtasks: ${task}`;
    tasks.push(task);
  }
  if (tasks.some((task) => task.query.trim().length === 0)) {
    return "spawn_subtasks: query is required";
  }
  return admissionError(tasks) ?? tasks;
}

/**
 * Starts a child for `task` and runs it to completion. The result text is
 * `Subtask <id> <outcome>. <last assistant text>` plus the child's chunk
 * refs; any outcome other than `done` is an error.
 */
async function runTask(
  opts: SubtaskToolsOptions,
  parentSessionId: string,
  task: TaskInput,
  signal: AbortSignal | undefined,
): Promise<TaskResult> {
  const child = startChild(opts, parentSessionId, task);
  if (!("session" in child)) return child;
  const { session, sessionId } = child;
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

/**
 * Creates the child session under a new id at the parent's current leaf
 * and appends the `subtask` entry as the child's first entry. The model is
 * the task's, else the parent's latest `request_header` model. `readOnly`
 * installs a guard that denies every tool whose `isReadOnly` is false; the
 * tool list itself is unchanged. Returns a failed result when the parent
 * has no model or no entries, or the session constructor throws.
 */
function startChild(
  opts: SubtaskToolsOptions,
  parentSessionId: string,
  task: TaskInput,
): { session: AgentSession; sessionId: string } | TaskResult {
  const budget = task.budget ?? DEFAULT_SUBTASK_BUDGET;
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
  return { session, sessionId };
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
      usage: emptyUsage(),
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

function modelFromLog(
  store: LogStore,
  sessionId: string,
): ModelRef | undefined {
  const header = lastRequestHeader(store.path(sessionId));
  return header
    ? { provider: header.provider, model: header.model }
    : undefined;
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

/** `child:<sessionId>#<chunkId>` per chunk on the child's last manifest. */
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
