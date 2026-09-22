import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { expect, it } from "vitest";
import type { Manifest, StoredEntry } from "../../src/index.ts";
import {
  expectFile,
  loadCase,
  replaySession,
} from "../../src/testing/index.ts";
import { caseDir, REPLAY_CASES } from "../replay/cases.ts";

function pretty(value: unknown): string {
  const raw = `${JSON.stringify(value, null, 2)}\n`;
  const formatted = spawnSync(
    path.join(process.cwd(), "node_modules/.bin/biome"),
    ["format", "--stdin-file-path=expected.json"],
    { input: raw, encoding: "utf8" },
  );
  if (formatted.status !== 0) {
    throw new Error(formatted.stderr || "biome format failed");
  }
  return formatted.stdout;
}

function normalize(entries: readonly StoredEntry[]): unknown[] {
  return entries.map((entry) => {
    const details = entry.details ? { ...entry.details } : undefined;
    if (details) delete details.responseId;
    const payload =
      entry.kind === "manifest"
        ? stabilize(entry.payload as Manifest, entries)
        : entry.payload;
    const next: Record<string, unknown> = {
      seq: entry.seq,
      kind: entry.kind,
      payload,
    };
    if (details && Object.keys(details).length > 0) next.details = details;
    return next;
  });
}

// Chunk ids are log entry ids, which change on every replay. Expected
// manifests keep seq:N so the committed file stays byte-stable.
function stabilize(manifest: Manifest, log: readonly StoredEntry[]): Manifest {
  const ids = new Map(log.map((entry) => [entry.id, entry.seq]));
  const swap = (value: string) => {
    if (ids.has(value)) return `seq:${ids.get(value)}`;
    let next = value;
    for (const [id, seq] of ids) {
      if (next.includes(id)) next = next.replaceAll(id, `seq:${seq}`);
    }
    return next;
  };
  return {
    ...manifest,
    chunks: manifest.chunks.map((chunk) => ({ ...chunk, id: swap(chunk.id) })),
    anomalies: manifest.anomalies.map((anomaly) =>
      anomaly.id === undefined ? anomaly : { ...anomaly, id: swap(anomaly.id) },
    ),
  };
}

it("writes a missing expected file and prints a diff on mismatch", () => {
  const previous = process.env.UPDATE_EXPECTED;
  delete process.env.UPDATE_EXPECTED;
  const dir = mkdtempSync(path.join(tmpdir(), "sage-expect-"));
  try {
    const file = path.join(dir, "nested", "system-prompt.md");
    expectFile(file, "alpha\n");
    expect(readFileSync(file, "utf8")).toBe("alpha\n");
    expectFile(file, "alpha\n");
    let message = "";
    try {
      expectFile(file, "beta\n");
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
    }
    expect(message).toContain("Expected file differs");
    expect(message).toContain("--- expected");
    expect(message).toContain("-alpha");
    expect(message).toContain("+beta");

    process.env.UPDATE_EXPECTED = "1";
    expectFile(file, "beta\n");
    expect(readFileSync(file, "utf8")).toBe("beta\n");
  } finally {
    if (previous === undefined) delete process.env.UPDATE_EXPECTED;
    else process.env.UPDATE_EXPECTED = previous;
    rmSync(dir, { recursive: true, force: true });
  }
});

it("refuses the record lane without calling the network", async () => {
  process.env.SAGE_RECORD = "1";
  try {
    await expect(
      replaySession(caseDir("text-only"), { system: "x", tools: [] }),
    ).rejects.toThrow(/#16/);
  } finally {
    delete process.env.SAGE_RECORD;
  }
});

for (const spec of REPLAY_CASES) {
  it(`replays ${spec.name} against the committed recording and expected files`, async () => {
    const dir = caseDir(spec.name);
    const result = await replaySession(dir, {
      system: spec.system,
      tools: spec.tools,
    });
    if (process.env.UPDATE_EXPECTED === "1") {
      writeFileSync(
        path.join(dir, "session.jsonl"),
        `${result.log.map((entry) => JSON.stringify(entry)).join("\n")}\n`,
      );
    }
    const loaded = loadCase(dir);
    expect(result.requests).toHaveLength(loaded.responses.length);
    expect(result.manifests).toHaveLength(loaded.responses.length);
    expect(normalize(result.log)).toEqual(normalize(loaded.log));

    const seen = result.requests[0];
    if (!seen) throw new Error("missing request");
    expectFile(
      path.join(loaded.expectedDir, "system-prompt.md"),
      seen.head.system.endsWith("\n")
        ? seen.head.system
        : `${seen.head.system}\n`,
    );
    expectFile(
      path.join(loaded.expectedDir, "tool-schemas.json"),
      pretty(seen.head.tools),
    );
    result.requests.forEach((request, index) => {
      expectFile(
        path.join(loaded.expectedDir, `turn-${index + 1}.request.json`),
        pretty(request),
      );
    });
    result.manifests.forEach((manifest, index) => {
      expectFile(
        path.join(loaded.expectedDir, `manifest-turn-${index + 1}.json`),
        pretty(stabilize(manifest, result.log)),
      );
    });
  });
}
