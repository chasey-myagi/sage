/**
 * expand tool. Reads a chunk's original text back from the Log.
 *
 * Contract: name `expand`, isReadOnly, accesses empty. `execute({ id })`
 * reads `store.path(sessionId)` and resolves ids with the same rules as
 * `indexChunks`. `seq:<seq>#<toolUseId>` returns that tool_result content.
 * `seq:<seq>` returns the prose of that message when it is a prose chunk.
 * `child:<sessionId>#<chunkId>` reads that session. An `own:` prefix is
 * stripped and resolved on the child path. Any other chunk id follows the
 * child's `subtask` entry to the parent leaf, so a parent `seq:` id still
 * names the original text. Anything else is isError `Unknown chunk: <id>`.
 * This tool does not append.
 *
 * Intentionally not in the Log: the tool object. The kernel logs the call
 * as an ordinary tool_use / tool_result. That pair is the expand numerator.
 */

import {
  asEntryId,
  type HarnessTool,
  isRecord,
  type LogStore,
  type StoredEntry,
} from "@sage/core";
import { OWN_CHUNK_PREFIX } from "./chunks.js";

/**
 * Model-visible name of the expand tool. The Log records tool calls by
 * name only, so readers of the Log match on this constant.
 */
export const EXPAND_TOOL_NAME = "expand";

const inputSchema = {
  type: "object",
  properties: {
    id: {
      type: "string",
      description: "Chunk id from an omitted stub, such as seq:3#toolu_1.",
    },
  },
  required: ["id"],
};

/**
 * Creates the `expand` tool bound to one session's Log. Unknown ids are an
 * `isError` result, not a throw.
 */
export function expandTool(opts: {
  store: LogStore;
  sessionId: string;
}): HarnessTool {
  return {
    def: {
      name: EXPAND_TOOL_NAME,
      description:
        "Retrieve the original text of an omitted chunk by its id. Does not re-run the tool.",
      inputSchema,
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute(input) {
      const id = readId(input);
      const text = id ? resolve(opts.store, opts.sessionId, id) : undefined;
      if (text === undefined) {
        return { content: `Unknown chunk: ${id}`, isError: true };
      }
      return { content: text, isError: false };
    },
  };
}

function readId(input: unknown): string {
  if (!isRecord(input) || typeof input.id !== "string") return "";
  return input.id;
}

function resolve(
  store: LogStore,
  sessionId: string,
  id: string,
): string | undefined {
  const child = /^child:([^#]+)#([\s\S]+)$/.exec(id);
  if (!child) return locate(store.path(sessionId), id);
  const childId = child[1] ?? "";
  const chunkId = child[2] ?? "";
  const path = store.path(childId);
  if (chunkId.startsWith(OWN_CHUNK_PREFIX)) {
    return locate(path, chunkId.slice(OWN_CHUNK_PREFIX.length));
  }
  const parent = parentPath(store, path);
  if (parent) {
    const found = locate(parent, chunkId);
    if (found !== undefined) return found;
  }
  return locate(path, chunkId);
}

function parentPath(
  store: LogStore,
  path: readonly StoredEntry[],
): readonly StoredEntry[] | undefined {
  const entry = path.find((item) => item.kind === "subtask");
  if (!entry || !isRecord(entry.payload)) return undefined;
  const parentSessionId = entry.payload.parentSessionId;
  const parentLeaf = entry.payload.parentLeaf;
  if (typeof parentSessionId !== "string") return undefined;
  if (typeof parentLeaf !== "string") return store.path(parentSessionId);
  return store.path(parentSessionId, asEntryId(parentLeaf));
}

function locate(path: readonly StoredEntry[], id: string): string | undefined {
  const tool = /^seq:(-?\d+)#([\s\S]+)$/.exec(id);
  if (tool) {
    const toolUseId = tool[2] ?? "";
    const message = messageBySeq(path, Number(tool[1]));
    if (!message) return undefined;
    const use = message.content.find(
      (block) => block.type === "tool_use" && block.id === toolUseId,
    );
    if (!use) return undefined;
    return firstResult(path, toolUseId) ?? "";
  }
  const prose = /^seq:(-?\d+)$/.exec(id);
  if (!prose) return undefined;
  const message = messageBySeq(path, Number(prose[1]));
  if (!message || !isProseChunk(message)) return undefined;
  return proseText(message);
}

type Block = {
  type: string;
  id?: string;
  text?: string;
  toolUseId?: string;
  content?: string;
};

type LoggedMessage = { role: string; content: Block[] };

function messageBySeq(
  path: readonly StoredEntry[],
  seq: number,
): LoggedMessage | undefined {
  for (const entry of path) {
    if (entry.kind !== "message" || entry.seq !== seq) continue;
    return asMessage(entry);
  }
  return undefined;
}

function firstResult(
  path: readonly StoredEntry[],
  toolUseId: string,
): string | undefined {
  for (const entry of path) {
    if (entry.kind !== "message") continue;
    const message = asMessage(entry);
    if (!message) continue;
    for (const block of message.content) {
      if (block.type === "tool_result" && block.toolUseId === toolUseId) {
        return typeof block.content === "string" ? block.content : "";
      }
    }
  }
  return undefined;
}

function isProseChunk(message: LoggedMessage): boolean {
  const onlyResults =
    message.content.length > 0 &&
    message.content.every((block) => block.type === "tool_result");
  if (onlyResults) return false;
  const toolUses = message.content.filter((block) => block.type === "tool_use");
  return proseText(message).length > 0 || toolUses.length === 0;
}

function proseText(message: LoggedMessage): string {
  let text = "";
  for (const block of message.content) {
    if (
      (block.type === "text" || block.type === "thinking") &&
      typeof block.text === "string"
    ) {
      text += block.text;
    }
  }
  return text;
}

function asMessage(entry: StoredEntry): LoggedMessage | undefined {
  if (!isRecord(entry.payload)) return undefined;
  const { role, content } = entry.payload;
  if (role !== "user" && role !== "assistant") return undefined;
  if (!Array.isArray(content)) return undefined;
  return { role, content: content as Block[] };
}
