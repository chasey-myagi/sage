import type { Manifest, StoredEntry } from "@sage/core";
import { replaySession } from "@sage/core/testing";
import { expect, it } from "vitest";
import { caseDir, REPLAY_CASES } from "../../../core/test/replay/cases.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";

function stabilize(manifest: Manifest, log: readonly StoredEntry[]): Manifest {
  const ids = new Map(log.map((entry) => [entry.id, entry.seq]));
  const swap = (value: string) => {
    if (ids.has(value)) return `seq:${ids.get(value)}`;
    return value;
  };
  return {
    ...manifest,
    chunks: manifest.chunks.map((chunk) => ({ ...chunk, id: swap(chunk.id) })),
  };
}

for (const spec of REPLAY_CASES) {
  it(`compiles replay case ${spec.name} to the passthrough request plus an empty budget`, async () => {
    const dir = caseDir(spec.name);
    const plain = await replaySession(dir, {
      system: spec.system,
      tools: spec.tools,
    });
    const compiled = await replaySession(dir, {
      system: spec.system,
      tools: spec.tools,
      viewProvider: contextCompiler({ budget: 8000, claims: [] }),
    });

    expect(compiled.requests).toEqual(plain.requests);
    expect(compiled.log.some((entry) => entry.kind === "view_epoch")).toBe(
      false,
    );
    expect(compiled.manifests).toHaveLength(plain.manifests.length);
    compiled.manifests.forEach((manifest, index) => {
      const plainManifest = plain.manifests[index];
      if (!plainManifest) throw new Error(`missing manifest ${index}`);
      const { budget, ...rest } = stabilize(manifest, compiled.log);
      const { budget: plainBudget, ...plainRest } = stabilize(
        plainManifest,
        plain.log,
      );
      expect(plainBudget).toBeUndefined();
      expect(rest).toEqual(plainRest);
      expect(budget).toEqual({ total: 8000, used: 0, overrun: 0 });
    });
  });
}
