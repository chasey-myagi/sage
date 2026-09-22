import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { afterEach, describe, expect, it } from "vitest";
import { type BashExec, createTools } from "../src/index.ts";

const ctx = { sessionId: "s", toolUseId: "tool-net" };
const PROFILE =
  '(version 1)(allow default)(deny network*)(allow network* (local ip "localhost:*"))';
const LOCAL_HTTP =
  'node -e \'const http=require("node:http");const s=http.createServer((q,r)=>r.end("ok"));s.listen(0,"127.0.0.1",()=>{const addr=s.address();if(!addr||typeof addr==="string")process.exit(1);http.get({host:"127.0.0.1",port:addr.port},(res)=>{const chunks=[];res.on("data",(c)=>chunks.push(c));res.on("end",()=>{s.close();process.exit(Buffer.concat(chunks).toString()==="ok"?0:1);});}).on("error",()=>process.exit(1));});\'';

let dir = "";

function freshDir(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-tools-net-"));
  return dir;
}

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

function tool(tools: HarnessTool[], name: string): HarnessTool {
  const found = tools.find((item) => item.def.name === name);
  if (!found) throw new Error(`missing tool ${name}`);
  return found;
}

type SpawnOpts = {
  argv?: readonly string[];
  shell?: boolean;
};

function watchingExec(seen: SpawnOpts[]): BashExec {
  return async (_command, opts) => {
    const extra = opts as SpawnOpts;
    seen.push({ argv: extra.argv, shell: extra.shell });
    return { stdout: "", stderr: "", code: 0 };
  };
}

describe("bash network policy", () => {
  it.skipIf(process.platform !== "darwin")(
    "denies external hosts and still reaches localhost when network is none",
    async () => {
      const cwd = freshDir();
      const bash = tool(createTools({ cwd, network: "none" }), "bash");
      const curl = await bash.execute(
        { command: "curl -s --max-time 5 https://api.github.com/" },
        ctx,
      );
      expect(curl.isError).toBe(true);
      const code = /^\[bash exit code (\d+)\]/.exec(curl.content)?.[1];
      const body = curl.content.replace(/^\[bash exit code \d+\]\n?/, "");
      const blocked =
        /could not resolve host/i.test(body) ||
        body.includes("000") ||
        code === "6";
      expect(blocked, curl.content).toBe(true);
      expect(curl.content).not.toContain("current_user_url");
      expect(curl.details).toEqual({ network: "none" });

      const local = await bash.execute({ command: LOCAL_HTTP }, ctx);
      expect(local.isError).toBe(false);
      expect(local.content).toMatch(/^\[bash exit code 0\]/);
      expect(local.details).toEqual({ network: "none" });
    },
    20_000,
  );

  it("does not wrap argv when network is full", async () => {
    const cwd = freshDir();
    const seen: SpawnOpts[] = [];
    const bash = tool(
      createTools({ cwd, network: "full", exec: watchingExec(seen) }),
      "bash",
    );
    const result = await bash.execute({ command: "echo hi" }, ctx);
    expect(result.isError).toBe(false);
    expect(result.details).toBeUndefined();
    expect(seen).toEqual([{ argv: ["echo hi"], shell: true }]);
  });

  it("wraps darwin argv with sandbox-exec and does not run it", async () => {
    const cwd = freshDir();
    const seen: SpawnOpts[] = [];
    const bash = tool(
      createTools({
        cwd,
        network: "none",
        platform: "darwin",
        exec: watchingExec(seen),
      }),
      "bash",
    );
    const result = await bash.execute({ command: "echo hi" }, ctx);
    expect(seen).toEqual([
      {
        argv: ["sandbox-exec", "-p", PROFILE, "sh", "-c", "echo hi"],
        shell: false,
      },
    ]);
    expect(result.details).toEqual({ network: "none" });
  });

  it("wraps linux argv with unshare -n when that helper is available", async () => {
    const cwd = freshDir();
    const seen: SpawnOpts[] = [];
    const bash = tool(
      createTools({
        cwd,
        network: "none",
        platform: "linux",
        unshareAvailable: true,
        exec: watchingExec(seen),
      }),
      "bash",
    );
    await bash.execute({ command: "echo hi" }, ctx);
    expect(seen).toEqual([
      {
        argv: ["unshare", "-n", "sh", "-c", "echo hi"],
        shell: false,
      },
    ]);
  });

  it("refuses network none on a platform other than darwin or linux", () => {
    const cwd = freshDir();
    expect(() =>
      createTools({ cwd, network: "none", platform: "win32" }),
    ).toThrow("tool network none is not supported on win32");
  });

  it("refuses network none on linux when unshare -n is unavailable", () => {
    const cwd = freshDir();
    expect(() =>
      createTools({
        cwd,
        network: "none",
        platform: "linux",
        unshareAvailable: false,
      }),
    ).toThrow("tool network none requires unshare -n, which is not available");
  });
});
