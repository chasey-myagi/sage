import type { Manifest, StoredEntry } from "@sage/core";
import { replaySession } from "@sage/core/testing";
import { expect, it } from "vitest";
import { caseDir, REPLAY_CASES } from "../../../core/test/replay/cases.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";

function expectedChunks(name: string): Manifest["chunks"][] {
  if (name === "text-only") {
    return [[{ id: "seq:0", tier: "full", reason: "kept", tokens: 2 }]];
  }
  if (name === "tool-roundtrip") {
    return [
      [{ id: "seq:0", tier: "full", reason: "kept", tokens: 1 }],
      [
        { id: "seq:0", tier: "full", reason: "kept", tokens: 1 },
        { id: "seq:3#call_1", tier: "full", reason: "recent", tokens: 5 },
      ],
    ];
  }
  throw new Error(`no chunk expectation for ${name}`);
}

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
  it(`compiles replay case ${spec.name} to the passthrough request and the new chunk index`, async () => {
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

    compiled.requests.forEach((request, index) => {
      const plainRequest = plain.requests[index];
      if (!plainRequest) throw new Error(`missing request ${index}`);
      expect(request.messages).toEqual(plainRequest.messages);
      expect(request.head).toEqual(plainRequest.head);
      expect(request.model).toEqual(plainRequest.model);
      expect(request.sampling).toEqual(plainRequest.sampling);
      const last = request.messages.length - 1;
      expect(request.cacheBreakpoints[0]).toBe("head");
      expect(request.cacheBreakpoints.at(-1)).toEqual({ afterMessage: last });
      expect(request.cacheBreakpoints).toHaveLength(3);
    });
    expect(compiled.log.some((entry) => entry.kind === "view_epoch")).toBe(
      false,
    );
    expect(compiled.manifests).toHaveLength(plain.manifests.length);
    compiled.manifests.forEach((manifest, index) => {
      const plainManifest = plain.manifests[index];
      if (!plainManifest) throw new Error(`missing manifest ${index}`);
      const {
        budget,
        chunks,
        epochDecision: _epochDecision,
        calibration: _calibration,
        cache: _cache,
        ...rest
      } = stabilize(manifest, compiled.log);
      const {
        budget: plainBudget,
        chunks: plainChunks,
        ...plainRest
      } = stabilize(plainManifest, plain.log);
      expect(plainBudget).toBeUndefined();
      expect(plainChunks.every((chunk) => chunk.reason === "passthrough")).toBe(
        true,
      );
      expect(rest).toEqual(plainRest);
      expect(chunks).toEqual(expectedChunks(spec.name)[index]);
      const kept = chunks.reduce(
        (sum, chunk) => sum + (chunk.tier === "none" ? 0 : (chunk.tokens ?? 0)),
        0,
      );
      expect(budget).toEqual({
        total: 8000,
        used: kept,
        overrun: 0,
        omitted: 0,
        expands: 0,
        frozenTokens: index === 0 ? 0 : 1,
        staleOmittable: 0,
      });
    });
  });
}
