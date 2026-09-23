/**
 * Validates spawn tool input and resolves presets into one task.
 *
 * Contract: tool input is untrusted model JSON. A task is
 * `{ query, budget?, readOnly?, model?, accesses? }` or
 * `{ preset, args? }` plus those fields. Explicit query, budget, readOnly,
 * and model override the preset; `{{args.name}}` placeholders are filled
 * from `args`, and a missing arg becomes empty. Malformed fields are
 * dropped, except a malformed `accesses`, which rejects the whole task.
 * Rejections are returned as values, never thrown.
 *
 * Intentionally not in the Log: the parsed task. The child's `subtask`
 * entry records the resolved query, budget, and readOnly.
 */

import { type Access, isRecord } from "@sage/core";
import type { ModelRef } from "@sage/provider/contract";
import {
  loadPresets,
  type PresetFs,
  renderPresetQuery,
  type SubtaskPreset,
} from "./presets.js";

/** One subtask request after validation. */
export type TaskInput = {
  /** "" until a preset supplies it. */
  query: string;
  budget?: number;
  readOnly?: boolean;
  model?: ModelRef;
  /** Preset stem to resolve; absent after `applyPreset`. */
  preset?: string;
  args?: Record<string, string>;
  /** Declared resources; absent on a writing task means an unknown write. */
  accesses?: Access[];
};

/** JSON schema of one task, shared by both spawn tools. */
export const TASK_SCHEMA = {
  type: "object",
  properties: {
    query: {
      type: "string",
      description:
        "Question for the child compile. It is appended after the shared prefix. Overrides a preset template.",
    },
    preset: {
      type: "string",
      description:
        "Stem of a .sage/subtasks markdown preset. Project files override the user directory.",
    },
    args: {
      type: "object",
      description: "Values substituted for {{args.name}} in the preset body.",
    },
    accesses: {
      type: "array",
      description:
        "Resources this task reads or writes. Omitted on a task that is not read-only means an unknown write.",
      items: {
        type: "object",
        properties: {
          key: { type: "string" },
          mode: { type: "string", enum: ["read", "write", "search"] },
          recursive: { type: "boolean" },
        },
        required: ["key", "mode"],
      },
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
};

/**
 * Parses one task. Returns undefined when the input has neither a string
 * `query` nor a non-empty `preset`, or when `accesses` is malformed.
 */
export function readTask(input: unknown): TaskInput | undefined {
  if (!isRecord(input)) return undefined;
  const hasQuery = typeof input.query === "string";
  const preset =
    typeof input.preset === "string" && input.preset.length > 0
      ? input.preset
      : undefined;
  if (!hasQuery && !preset) return undefined;
  const task: TaskInput = { query: hasQuery ? String(input.query) : "" };
  if (preset) task.preset = preset;
  if (typeof input.budget === "number" && Number.isFinite(input.budget)) {
    task.budget = input.budget;
  }
  if (typeof input.readOnly === "boolean") task.readOnly = input.readOnly;
  const model = readModel(input.model);
  if (model) task.model = model;
  const args = readArgs(input.args);
  if (args) task.args = args;
  if ("accesses" in input) {
    const accesses = readAccesses(input.accesses);
    if (!accesses) return undefined;
    task.accesses = accesses;
  }
  return task;
}

/** Parses `{ tasks }`; undefined when any task fails `readTask`. */
export function readTasks(input: unknown): TaskInput[] | undefined {
  if (!isRecord(input) || !Array.isArray(input.tasks)) return undefined;
  const tasks: TaskInput[] = [];
  for (const value of input.tasks) {
    const task = readTask(value);
    if (!task) return undefined;
    tasks.push(task);
  }
  return tasks;
}

/**
 * Loads presets from `cwd` and `home`; an empty map without `cwd`. Reads
 * the directories on every call, so edits apply to the next spawn.
 */
export function presetsOf(opts: {
  cwd?: string;
  home?: string;
  fs?: PresetFs;
}): Map<string, SubtaskPreset> {
  if (!opts.cwd) return new Map();
  return loadPresets({
    cwd: opts.cwd,
    ...(opts.home ? { home: opts.home } : {}),
    ...(opts.fs ? { fs: opts.fs } : {}),
  });
}

/**
 * Resolves `task.preset` into a task with a query. Returns an error string
 * for an unknown preset or a query that is still blank.
 */
export function applyPreset(
  task: TaskInput,
  presets: Map<string, SubtaskPreset>,
): TaskInput | string {
  if (!task.preset) {
    if (task.query.trim().length === 0) return "query is required";
    return task;
  }
  const preset = presets.get(task.preset);
  if (!preset) return `Unknown preset: ${task.preset}`;
  const rendered = renderPresetQuery(preset.query, task.args ?? {});
  const query = task.query.trim().length > 0 ? task.query : rendered;
  if (query.trim().length === 0) return "query is required";
  const resolved: TaskInput = { query };
  if (task.budget !== undefined) resolved.budget = task.budget;
  else if (preset.budget !== undefined) resolved.budget = preset.budget;
  if (task.readOnly !== undefined) resolved.readOnly = task.readOnly;
  else if (preset.readOnly !== undefined) resolved.readOnly = preset.readOnly;
  if (task.model) resolved.model = task.model;
  else if (preset.model) resolved.model = preset.model;
  if (task.accesses) resolved.accesses = task.accesses;
  return resolved;
}

function readAccesses(value: unknown): Access[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const accesses: Access[] = [];
  for (const item of value) {
    if (!isRecord(item) || typeof item.key !== "string") return undefined;
    if (
      item.mode !== "read" &&
      item.mode !== "write" &&
      item.mode !== "search"
    ) {
      return undefined;
    }
    const access: Access = { key: item.key, mode: item.mode };
    if (typeof item.recursive === "boolean") access.recursive = item.recursive;
    accesses.push(access);
  }
  return accesses;
}

function readArgs(value: unknown): Record<string, string> | undefined {
  if (!isRecord(value)) return undefined;
  const args: Record<string, string> = {};
  for (const [key, item] of Object.entries(value)) {
    if (typeof item === "string") args[key] = item;
    else if (typeof item === "number" && Number.isFinite(item)) {
      args[key] = String(item);
    } else if (typeof item === "boolean") args[key] = String(item);
  }
  return args;
}

function readModel(value: unknown): ModelRef | undefined {
  if (!isRecord(value)) return undefined;
  if (typeof value.provider !== "string" || typeof value.model !== "string") {
    return undefined;
  }
  return { provider: value.provider, model: value.model };
}
