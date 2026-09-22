import type { ToolDef } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { toolCatalog } from "../../src/claims/index.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { contextOf, runSession, stopScript } from "./session.ts";

const deferred: ToolDef[] = [
  {
    name: "special",
    description: "Do a special thing",
    inputSchema: { type: "object", properties: { q: { type: "string" } } },
  },
  {
    name: "other",
    description: "Do the other thing",
    inputSchema: { type: "object" },
  },
];

it("lists deferred tools as a line catalog and does not put them on the head", async () => {
  const { store, sessionId } = await runSession({
    scripts: [stopScript("ok")],
  });
  const ctx = contextOf(store, sessionId);
  expect(await toolCatalog({ deferred: [] })(ctx)).toEqual([]);

  const claims = await toolCatalog({ deferred })(ctx);
  const line = "- special — Do a special thing\n- other — Do the other thing";
  const full = [
    line,
    "",
    "special",
    JSON.stringify(deferred[0]?.inputSchema, null, 2),
    "",
    "other",
    JSON.stringify(deferred[1]?.inputSchema, null, 2),
  ].join("\n");
  expect(claims).toEqual([
    {
      id: "tool-catalog",
      priority: 40,
      placement: "epoch-head",
      tiers: { line, full },
    },
  ]);

  const view = await contextCompiler({
    budget: 40000,
    claims: [toolCatalog({ deferred })],
  }).view(ctx);
  expect(view.head.tools).toEqual([]);
  const shown = view.messages[0]?.content
    .map((block) => (block.type === "text" ? block.text : ""))
    .join("\n");
  expect(shown).toContain(line);
  expect(view.head.tools.map((tool) => tool.name)).not.toContain("special");
});
