import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  InvalidEntryError,
  JsonlLogStore,
  MemoryLogStore,
  registerEntryKind,
  UnknownEntryKindError,
} from "../../src/index.ts";

const dirs: string[] = [];

afterEach(() => {
  for (const dir of dirs.splice(0))
    rmSync(dir, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "sage-log-"));
  dirs.push(dir);
  return dir;
}

describe("entry kinds", () => {
  it("refuses an unregistered kind on open and does not skip the line", () => {
    const dir = tempDir();
    const sessionId = "unk";
    mkdirSync(dir, { recursive: true });
    const good = {
      id: "11111111-1111-1111-1111-111111111111",
      parentId: null,
      seq: 0,
      at: "2026-09-22T00:00:00.000Z",
      kind: "message",
      payload: { role: "user", content: "ok" },
    };
    const bad = {
      id: "22222222-2222-2222-2222-222222222222",
      parentId: good.id,
      seq: 1,
      at: "2026-09-22T00:00:01.000Z",
      kind: "not-a-kind",
      payload: {},
    };
    const later = {
      id: "33333333-3333-3333-3333-333333333333",
      parentId: bad.id,
      seq: 2,
      at: "2026-09-22T00:00:02.000Z",
      kind: "message",
      payload: { role: "user", content: "later" },
    };
    writeFileSync(
      join(dir, `${sessionId}.jsonl`),
      `${JSON.stringify(good)}\n${JSON.stringify(bad)}\n${JSON.stringify(later)}\n`,
    );

    const store = new JsonlLogStore({ dir });
    expect(() => store.all(sessionId)).toThrow(UnknownEntryKindError);
    try {
      store.all(sessionId);
      throw new Error("expected unknown kind to throw");
    } catch (error) {
      expect(error).toBeInstanceOf(UnknownEntryKindError);
      expect((error as UnknownEntryKindError).message).toContain("not-a-kind");
      expect((error as UnknownEntryKindError).kind).toBe("not-a-kind");
    }
  });

  it("rejects an invalid payload before writing the file", () => {
    const dir = tempDir();
    const sessionId = "inv";
    const store = new JsonlLogStore({ dir });
    store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "ok" },
    });
    const file = join(dir, `${sessionId}.jsonl`);
    const before = readFileSync(file, "utf8");

    expect(() =>
      store.append(sessionId, {
        kind: "message",
        payload: { nope: true },
      }),
    ).toThrow(InvalidEntryError);
    expect(readFileSync(file, "utf8")).toBe(before);
    expect(store.all(sessionId)).toHaveLength(1);

    expect(() =>
      store.append("fresh", { kind: "message", payload: {} }),
    ).toThrow(InvalidEntryError);
    expect(existsSync(join(dir, "fresh.jsonl"))).toBe(false);
  });

  it("accepts each kernel kind shape and rejects a broken payload", () => {
    const store = new MemoryLogStore();
    const sessionId = "shapes";
    const message = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: [{ type: "text", text: "hi" }] },
    });
    expect(() =>
      store.append(sessionId, { kind: "message", payload: { role: 1 } }),
    ).toThrow(InvalidEntryError);

    store.append(sessionId, {
      kind: "request_header",
      payload: {
        provider: "anthropic",
        model: "claude",
        systemPromptHash: "s",
        toolsHash: "t",
        sampling: { temperature: 0 },
        adapterDefaults: {},
      },
    });
    expect(() =>
      store.append(sessionId, {
        kind: "request_header",
        payload: {
          provider: "anthropic",
          model: "claude",
          systemPromptHash: "s",
          toolsHash: "t",
          sampling: {},
          adapterDefaults: {},
          system: 1,
        },
      }),
    ).toThrow(InvalidEntryError);

    store.append(sessionId, {
      kind: "view_epoch",
      payload: { epoch: 1, reason: "budget" },
    });
    store.append(sessionId, {
      kind: "model_change",
      payload: {
        from: { provider: "a", model: "b" },
        to: { provider: "a", model: "c" },
        reason: "switch",
      },
    });
    store.append(sessionId, {
      kind: "manifest",
      payload: {
        epoch: 1,
        turn: 1,
        budget: 10,
        claims: [],
        chunks: [],
        anomalies: [],
      },
    });
    store.append(sessionId, { kind: "undo", payload: { target: message.id } });
    const pointer = store.append(sessionId, {
      kind: "leaf",
      payload: { target: message.id },
    });
    expect(pointer.kind).toBe("leaf");
    expect(store.leaf(sessionId)).toBe(message.id);
    expect(() =>
      store.append(sessionId, { kind: "leaf", payload: { target: "missing" } }),
    ).toThrow(InvalidEntryError);
    store.append(sessionId, {
      kind: "terminal",
      payload: { outcome: "stop", usage: { output: 1 } },
    });
    expect(() =>
      store.append(sessionId, { kind: "terminal", payload: {} }),
    ).toThrow(InvalidEntryError);
  });

  it("accepts a Manifest-shaped payload and rejects one without anomalies", () => {
    const store = new MemoryLogStore();
    const sessionId = "manifest-shape";
    store.append(sessionId, {
      kind: "manifest",
      payload: {
        epoch: 1,
        turn: 2,
        claims: [],
        chunks: [],
        anomalies: [],
      },
    });
    expect(() =>
      store.append(sessionId, {
        kind: "manifest",
        payload: {
          epoch: 1,
          turn: 1,
          budget: null,
          claims: [],
          chunks: [],
        },
      }),
    ).toThrow(InvalidEntryError);
  });

  it("accepts a budget report object and rejects one missing overrun", () => {
    const store = new MemoryLogStore();
    const sessionId = "budget-report";
    store.append(sessionId, {
      kind: "manifest",
      payload: {
        epoch: 0,
        turn: 1,
        budget: { total: 10000, used: 12000, overrun: 2000 },
        claims: [],
        chunks: [],
        anomalies: [],
      },
    });
    expect(store.path(sessionId)).toHaveLength(1);
    expect(() =>
      store.append(sessionId, {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          budget: { total: 10000, used: 12000 },
          claims: [],
          chunks: [],
          anomalies: [],
        },
      }),
    ).toThrow(InvalidEntryError);
  });

  it("accepts an optional terminal health object and rejects a non-object", () => {
    const store = new MemoryLogStore();
    store.append("health-ok", {
      kind: "terminal",
      payload: { outcome: "done", health: { turns: 1, cacheHitRatio: null } },
    });
    expect(store.path("health-ok")).toHaveLength(1);
    expect(() =>
      store.append("health-bad", {
        kind: "terminal",
        payload: { outcome: "done", health: "nope" },
      }),
    ).toThrow(InvalidEntryError);
  });

  it("accepts manifest shadow and rejects a shadow object without scorer or calls", () => {
    const store = new MemoryLogStore();
    store.append("shadow-ok", {
      kind: "manifest",
      payload: {
        epoch: 0,
        turn: 1,
        claims: [],
        chunks: [],
        anomalies: [],
        shadow: {
          scorer: "rule",
          calls: 1,
          decisions: [{ id: "seq:1#t", tier: "none", reason: "shadow" }],
          usage: { inputOther: 1, inputCacheRead: 0, output: 1 },
          fallback: "boom",
        },
      },
    });
    expect(store.path("shadow-ok")).toHaveLength(1);
    expect(() =>
      store.append("shadow-bad", {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          claims: [],
          chunks: [],
          anomalies: [],
          shadow: { decisions: [] },
        },
      }),
    ).toThrow(InvalidEntryError);
    expect(() =>
      store.append("shadow-no-calls", {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          claims: [],
          chunks: [],
          anomalies: [],
          shadow: { scorer: "model", decisions: [] },
        },
      }),
    ).toThrow(InvalidEntryError);
  });

  it("accepts manifest entryScored and rejects an incomplete object", () => {
    const store = new MemoryLogStore();
    store.append("entry-ok", {
      kind: "manifest",
      payload: {
        epoch: 0,
        turn: 1,
        claims: [],
        chunks: [],
        anomalies: [],
        entryScored: {
          calls: 1,
          none: 1,
          summary: 0,
          full: 0,
          summaryRejected: 1,
          usage: { inputOther: 1, output: 2 },
        },
      },
    });
    expect(store.path("entry-ok")).toHaveLength(1);
    expect(() =>
      store.append("entry-bad", {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          claims: [],
          chunks: [],
          anomalies: [],
          entryScored: { calls: 1 },
        },
      }),
    ).toThrow(InvalidEntryError);
    expect(() =>
      store.append("entry-rejected-bad", {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          claims: [],
          chunks: [],
          anomalies: [],
          entryScored: {
            calls: 1,
            none: 0,
            summary: 0,
            full: 1,
            summaryRejected: "no",
          },
        },
      }),
    ).toThrow(InvalidEntryError);
  });

  it("rejects shadow and scorer raw that is not text plus finish", () => {
    const store = new MemoryLogStore();
    expect(() =>
      store.append("shadow-raw-bad", {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          claims: [],
          chunks: [],
          anomalies: [],
          shadow: { scorer: "model", decisions: [], raw: "nope" },
        },
      }),
    ).toThrow(InvalidEntryError);
    expect(() =>
      store.append("scorer-raw-bad", {
        kind: "manifest",
        payload: {
          epoch: 0,
          turn: 1,
          claims: [],
          chunks: [],
          anomalies: [],
          scorer: { calls: 1, fallback: true, raw: { text: 1 } },
        },
      }),
    ).toThrow(InvalidEntryError);
  });

  it("accepts shadow and scorer raw of text plus finish", () => {
    const store = new MemoryLogStore();
    const raw = { text: "not json", finish: "stop" };
    store.append("raw-ok", {
      kind: "manifest",
      payload: {
        epoch: 0,
        turn: 1,
        claims: [],
        chunks: [],
        anomalies: [],
        scorer: { calls: 1, fallback: true, raw },
        shadow: {
          scorer: "model",
          calls: 1,
          decisions: [],
          fallback: "invalid-json",
          raw,
        },
      },
    });
    expect(store.path("raw-ok")).toHaveLength(1);
  });

  it("throws when the same kind is registered twice", () => {
    expect(() =>
      registerEntryKind({
        kind: "message",
        persisted: true,
        doc: "duplicate",
        validate: (payload): payload is { role: string; content: unknown } =>
          typeof payload === "object" && payload !== null,
      }),
    ).toThrow(/already registered/);
  });
});
