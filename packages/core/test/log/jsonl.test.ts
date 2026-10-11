import {
  fsyncSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  writeSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { JsonlLogStore, LogCorruptedError } from "../../src/index.ts";
import { nodeLogFs } from "../../src/testing/index.ts";

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

describe("JsonlLogStore", () => {
  it("persists 100 JSON lines with monotonic seq and parent links", () => {
    const dir = tempDir();
    const sessionId = "s";
    const store = new JsonlLogStore({ dir });
    const appended = [];
    for (let i = 0; i < 100; i++) {
      appended.push(
        store.append(sessionId, {
          kind: "message",
          payload: { role: "user", content: `m${i}` },
        }),
      );
    }

    const file = readFileSync(join(dir, `${sessionId}.jsonl`), "utf8");
    const lines = file.split("\n").filter((line) => line.length > 0);
    expect(lines).toHaveLength(100);
    for (const line of lines) {
      expect(() => JSON.parse(line)).not.toThrow();
    }

    const reopened = new JsonlLogStore({ dir });
    expect(reopened.all(sessionId)).toHaveLength(100);
    for (let i = 0; i < 100; i++) {
      const entry = reopened.all(sessionId)[i];
      expect(entry?.seq).toBe(i);
      expect(entry?.parentId).toBe(i === 0 ? null : appended[i - 1]?.id);
    }
    expect(reopened.leaf(sessionId)).toBe(appended[99]?.id);
  });

  it("writes each entry once and never forces a disk sync", () => {
    const dir = tempDir();
    let writes = 0;
    let syncs = 0;
    // Not annotated as LogFs: `fsyncSync` is an extra method the store must
    // never reach for.
    const fs = {
      ...nodeLogFs({
        writeSync: (fd, data) => {
          writes += 1;
          return writeSync(fd, data);
        },
      }),
      fsyncSync: (fd: number) => {
        syncs += 1;
        fsyncSync(fd);
      },
    };
    const store = new JsonlLogStore({ dir, fs });
    for (let i = 0; i < 100; i++) {
      store.append("nosync", {
        kind: "message",
        payload: { role: "user", content: `m${i}` },
      });
    }

    expect(writes).toBe(100);
    expect(syncs).toBe(0);
    expect(new JsonlLogStore({ dir }).all("nosync")).toHaveLength(100);
  });

  it("finishes a line the kernel wrote only part of before appending the next", () => {
    const dir = tempDir();
    const fs = nodeLogFs({
      // Every call is a short write of at most 7 bytes.
      writeSync: (fd, data) => writeSync(fd, data.subarray(0, 7)),
    });
    const store = new JsonlLogStore({ dir, fs });
    for (let i = 0; i < 3; i++) {
      store.append("short", {
        kind: "message",
        payload: { role: "user", content: `message ${i} ünïcode` },
      });
    }

    const reopened = new JsonlLogStore({ dir });
    expect(reopened.all("short").map((entry) => entry.payload)).toEqual(
      [0, 1, 2].map((i) => ({ role: "user", content: `message ${i} ünïcode` })),
    );
  });

  it("drops a torn last line without a watermark and continues at seq 56", () => {
    const dir = tempDir();
    const sessionId = "torn";
    const store = new JsonlLogStore({ dir });
    const appended = [];
    for (let i = 0; i < 100; i++) {
      appended.push(
        store.append(sessionId, {
          kind: "message",
          payload: { role: "user", content: `m${i}` },
        }),
      );
    }

    const file = join(dir, `${sessionId}.jsonl`);
    const lines = readFileSync(file, "utf8").replace(/\n$/, "").split("\n");
    const line57 = lines[56];
    if (!line57) throw new Error("missing line 57");
    const middle = line57.slice(0, Math.floor(line57.length / 2));
    writeFileSync(file, `${lines.slice(0, 56).join("\n")}\n${middle}`);

    const reopened = new JsonlLogStore({ dir });
    expect(reopened.all(sessionId)).toHaveLength(56);
    expect(reopened.torn(sessionId)).toBe(true);
    const next = reopened.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "after tear" },
    });
    expect(next.seq).toBe(56);
    expect(next.parentId).toBe(appended[55]?.id);
    expect(reopened.leaf(sessionId)).toBe(next.id);

    const persisted = new JsonlLogStore({ dir });
    expect(persisted.all(sessionId)).toHaveLength(57);
    expect(persisted.all(sessionId)[56]?.id).toBe(next.id);
    expect(persisted.torn(sessionId)).toBe(false);
  });

  it("throws LogCorruptedError with the line number for a bad middle line", () => {
    const dir = tempDir();
    const sessionId = "bad";
    const store = new JsonlLogStore({ dir });
    for (let i = 0; i < 40; i++) {
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: `m${i}` },
      });
    }

    const file = join(dir, `${sessionId}.jsonl`);
    const lines = readFileSync(file, "utf8").replace(/\n$/, "").split("\n");
    lines[29] = "not-json";
    writeFileSync(file, `${lines.join("\n")}\n`);

    const reopened = new JsonlLogStore({ dir });
    expect(() => reopened.all(sessionId)).toThrow(LogCorruptedError);
    try {
      reopened.all(sessionId);
    } catch (error) {
      expect(error).toBeInstanceOf(LogCorruptedError);
      expect((error as LogCorruptedError).message).toContain("30");
      expect((error as LogCorruptedError).line).toBe(30);
    }
  });

  it("throws and keeps memory and the leaf unchanged when a write makes no progress", () => {
    const dir = tempDir();
    const sessionId = "stalled";
    let writes = 0;
    const fs = nodeLogFs({
      writeSync: (fd, data) => {
        writes += 1;
        return writes >= 2 ? 0 : writeSync(fd, data);
      },
    });
    const store = new JsonlLogStore({ dir, fs });
    const first = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "kept" },
    });

    expect(() =>
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "stalled" },
      }),
    ).toThrow(/no progress/);
    expect(store.all(sessionId)).toHaveLength(1);
    expect(store.leaf(sessionId)).toBe(first.id);
  });

  it("leaves memory and the leaf unchanged when writeSync throws", () => {
    const dir = tempDir();
    const sessionId = "disk";
    let writes = 0;
    const fs = nodeLogFs({
      writeSync: (fd, data) => {
        writes += 1;
        if (writes >= 2) throw new Error("disk full");
        return writeSync(fd, data);
      },
    });
    const store = new JsonlLogStore({ dir, fs });
    const first = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "kept" },
    });

    expect(() =>
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "lost" },
      }),
    ).toThrow(/disk full/);

    expect(store.all(sessionId)).toHaveLength(1);
    expect(store.leaf(sessionId)).toBe(first.id);
    const text = readFileSync(join(dir, `${sessionId}.jsonl`), "utf8");
    expect(text).not.toContain("lost");
    expect(text).toContain("kept");
  });

  it.each([
    [
      "throws",
      (): number => {
        throw new Error("disk full");
      },
    ],
    ["makes no progress", (): number => 0],
  ])(
    "removes the partial line when a write %s mid-line so the next append starts a clean line",
    (_mode, fail) => {
      const dir = tempDir();
      const sessionId = "partial";
      let failing = false;
      let callsWhileFailing = 0;
      const fs = nodeLogFs({
        writeSync: (fd, data) => {
          if (!failing) return writeSync(fd, data);
          callsWhileFailing += 1;
          return callsWhileFailing === 1
            ? writeSync(fd, data.subarray(0, 5))
            : fail();
        },
      });
      const store = new JsonlLogStore({ dir, fs });
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "kept" },
      });
      failing = true;
      expect(() =>
        store.append(sessionId, {
          kind: "message",
          payload: { role: "user", content: "lost" },
        }),
      ).toThrow();
      failing = false;
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "after" },
      });

      const reopened = new JsonlLogStore({ dir });
      expect(reopened.torn(sessionId)).toBe(false);
      expect(
        reopened.all(sessionId).map((entry) => [entry.seq, entry.payload]),
      ).toEqual([
        [0, { role: "user", content: "kept" }],
        [1, { role: "user", content: "after" }],
      ]);
      expect(store.all(sessionId)).toEqual(reopened.all(sessionId));
    },
  );

  it("refuses later appends when the partial line of a failed write cannot be removed", () => {
    const dir = tempDir();
    const sessionId = "stuck";
    let failing = false;
    let callsWhileFailing = 0;
    const fs = nodeLogFs({
      ftruncateSync: () => {
        throw new Error("truncate failed");
      },
      writeSync: (fd, data) => {
        if (!failing) return writeSync(fd, data);
        callsWhileFailing += 1;
        if (callsWhileFailing === 1) return writeSync(fd, data.subarray(0, 5));
        throw new Error("disk full");
      },
    });
    const store = new JsonlLogStore({ dir, fs });
    const first = store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "kept" },
    });
    failing = true;
    expect(() =>
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "lost" },
      }),
    ).toThrow(/disk full/);
    failing = false;
    expect(() =>
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "after" },
      }),
    ).toThrow(
      expect.objectContaining({
        message: expect.stringMatching(/partial line/),
        cause: expect.objectContaining({ message: "truncate failed" }),
      }),
    );
    expect(store.all(sessionId)).toHaveLength(1);
    expect(store.leaf(sessionId)).toBe(first.id);

    const reopened = new JsonlLogStore({ dir });
    expect(reopened.torn(sessionId)).toBe(true);
    expect(reopened.all(sessionId).map((entry) => entry.payload)).toEqual([
      { role: "user", content: "kept" },
    ]);
  });

  it("truncates only its own partial line when another writer appended after its last line", () => {
    const dir = tempDir();
    const sessionId = "shared";
    let failing = false;
    let callsWhileFailing = 0;
    const fs = nodeLogFs({
      writeSync: (fd, data) => {
        if (!failing) return writeSync(fd, data);
        callsWhileFailing += 1;
        if (callsWhileFailing === 1) return writeSync(fd, data.subarray(0, 5));
        throw new Error("disk full");
      },
    });
    const store = new JsonlLogStore({ dir, fs });
    store.append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "kept" },
    });
    new JsonlLogStore({ dir }).append(sessionId, {
      kind: "message",
      payload: { role: "user", content: "other writer" },
    });
    failing = true;
    expect(() =>
      store.append(sessionId, {
        kind: "message",
        payload: { role: "user", content: "lost" },
      }),
    ).toThrow(/disk full/);

    const reopened = new JsonlLogStore({ dir });
    expect(reopened.torn(sessionId)).toBe(false);
    expect(reopened.all(sessionId).map((entry) => entry.payload)).toEqual([
      { role: "user", content: "kept" },
      { role: "user", content: "other writer" },
    ]);
  });

  it("round-trips opaque details without interpreting them", () => {
    const dir = tempDir();
    const store = new JsonlLogStore({ dir });
    const entry = store.append("details", {
      kind: "message",
      payload: { role: "user", content: "hi" },
      details: { claim: "c1", n: 1 },
    });

    expect(entry.details).toEqual({ claim: "c1", n: 1 });
    const reopened = new JsonlLogStore({ dir });
    expect(reopened.get("details", entry.id)?.details).toEqual({
      claim: "c1",
      n: 1,
    });
  });
});
