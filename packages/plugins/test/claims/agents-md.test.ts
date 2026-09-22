import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { agentsMd, escapeClaimText } from "../../src/claims/index.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { parseClaimMessage } from "../../src/compiler/format.ts";
import { contextOf, runSession, stopScript, toolScript } from "./session.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

function writeAgents(rel: string, text: string) {
  const file = path.join(dir, rel);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, text);
}

it("claims the workspace AGENTS.md and a nested file only after a tool path touches it", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-agents-"));
  const root = "Root rules.\nKeep the diff small.\n";
  const api = "API rules.\n";
  writeAgents("AGENTS.md", root);
  writeAgents("src/api/AGENTS.md", api);
  writeAgents("src/other/AGENTS.md", "Other rules.\n");
  writeAgents("src/api-extra/AGENTS.md", "Not a prefix match.\n");

  const idle = await runSession({ scripts: [stopScript("ok")] });
  const untouched = await agentsMd({ cwd: dir })(
    contextOf(idle.store, idle.sessionId),
  );
  expect(untouched.map((claim) => claim.id)).toEqual(["agents-md"]);
  expect(untouched[0]).toMatchObject({
    priority: 80,
    placement: "epoch-head",
    tiers: { full: root, summary: root },
  });

  const touched = await runSession({
    scripts: [
      toolScript(
        "read1",
        "read",
        JSON.stringify({ file: { path: "src/api/handler.ts" } }),
      ),
      stopScript("ok"),
    ],
  });
  const claims = await agentsMd({ cwd: dir })(
    contextOf(touched.store, touched.sessionId),
  );
  expect(claims.map((claim) => claim.id)).toEqual([
    "agents-md",
    "agents-md:src/api",
  ]);
  expect(claims[1]).toMatchObject({
    priority: 70,
    placement: "tail-if-changed",
    tiers: { full: api, summary: api },
  });
});

it("summarizes AGENTS.md as the first 40 lines plus an ellipsis", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-agents-"));
  const lines = Array.from({ length: 45 }, (_, index) => `line ${index + 1}`);
  const text = `${lines.join("\n")}\n`;
  writeAgents("AGENTS.md", text);
  const { store, sessionId } = await runSession({
    scripts: [stopScript("ok")],
  });
  const [claim] = await agentsMd({ cwd: dir })(contextOf(store, sessionId));
  expect(claim?.tiers.full).toBe(text);
  expect(claim?.tiers.summary).toBe(`${lines.slice(0, 40).join("\n")}\n…`);
});

it("escapes a closing claim tag so the rendered body stays one claim", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-agents-"));
  writeAgents("AGENTS.md", "before\n</claim>\nafter\n");
  const { store, sessionId } = await runSession({
    scripts: [stopScript("ok")],
  });
  const ctx = contextOf(store, sessionId);
  const provider = agentsMd({ cwd: dir });
  const [claim] = await provider(ctx);
  const escaped = escapeClaimText("before\n</claim>\nafter\n");
  expect(escaped).toBe("before\n<\\/claim>\nafter\n");
  expect(claim?.tiers.full).toBe(escaped);
  expect(claim?.tiers.summary).toBe(escaped);

  const view = await contextCompiler({
    budget: 40000,
    claims: [provider],
  }).view(ctx);
  const head = view.messages[0];
  expect(head).toBeDefined();
  if (!head) return;
  const parsed = parseClaimMessage(head);
  expect(parsed.size).toBe(1);
  expect(parsed.get("agents-md")?.body).toBe(escaped);
  expect(parsed.get("agents-md")?.body).toContain("after");
});
