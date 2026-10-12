import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createFaux, stopScript } from "@sage/core/testing";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BenchDeps } from "../src/bench/types.ts";
import { defaultRoutePolicy } from "../src/coding-session.ts";
import { runCli } from "../src/main.ts";

const GATEWAY = {
  name: "example-gateway",
  baseUrl: "https://gateway.example.com/v1",
  envKey: "EXAMPLE_GATEWAY_KEY",
  api: "openai-chat",
};

const CREDIT_PRICING = {
  inputPerMTok: 3,
  outputPerMTok: 12,
  cacheReadPerMTok: 0.3,
  cacheWritePerMTok: 3.75,
  currency: "credit",
  source: "https://gateway.example.com/pricing",
};

const FILE = {
  providers: [GATEWAY],
  models: [
    {
      provider: "example-gateway",
      model: "example-model",
      contextWindow: 200_000,
      pricing: CREDIT_PRICING,
    },
  ],
};

let dir = "";

afterEach(() => {
  vi.unstubAllEnvs();
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

/** A temp dir with `home/` (holding the providers file) and `work/`. */
function setup(file: unknown): { home: string; work: string; file: string } {
  dir = mkdtempSync(path.join(tmpdir(), "sage-local-providers-"));
  const home = path.join(dir, "home");
  const work = path.join(dir, "work");
  mkdirSync(path.join(home, ".sage"), { recursive: true });
  mkdirSync(work);
  const target = path.join(home, ".sage", "providers.json");
  if (typeof file === "string") writeFileSync(target, file);
  else if (file !== undefined) writeFileSync(target, JSON.stringify(file));
  return { home, work, file: target };
}

type Call = {
  url: string;
  authorization: string | null;
  apiKey: string | null;
};

const OPENAI_SSE = [
  'data: {"id":"c1","object":"chat.completion.chunk","model":"example-model","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}',
  "",
  'data: {"id":"c1","object":"chat.completion.chunk","model":"example-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}',
  "",
  'data: {"id":"c1","object":"chat.completion.chunk","model":"example-model","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1}}',
  "",
  "data: [DONE]",
  "",
].join("\n");

const ANTHROPIC_SSE = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"m1","model":"example-model","usage":{"input_tokens":3,"output_tokens":0}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}',
  "",
  "event: content_block_stop",
  'data: {"type":"content_block_stop","index":0}',
  "",
  "event: message_delta",
  'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
  "",
  "event: message_stop",
  'data: {"type":"message_stop"}',
  "",
].join("\n");

/** Runs `fn` with global fetch answering every call with `sse`. */
async function withFetch<T>(
  sse: string,
  fn: (calls: Call[]) => Promise<T>,
): Promise<T> {
  const calls: Call[] = [];
  const disabled = globalThis.fetch;
  globalThis.fetch = (async (input, init) => {
    const headers = new Headers(init?.headers);
    calls.push({
      url: String(input),
      authorization: headers.get("authorization"),
      apiKey: headers.get("x-api-key"),
    });
    return new Response(sse, {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  }) as typeof fetch;
  try {
    return await fn(calls);
  } finally {
    globalThis.fetch = disabled;
  }
}

function capture() {
  let stdout = "";
  let stderr = "";
  return {
    stdout: {
      write(chunk: string) {
        stdout += chunk;
      },
    },
    stderr: {
      write(chunk: string) {
        stderr += chunk;
      },
    },
    text: () => ({ stdout, stderr }),
  };
}

function requestHeaders(cwd: string): Record<string, unknown>[] {
  const sessions = path.join(cwd, ".sage", "sessions");
  const name = readdirSync(sessions).find((file) => file.endsWith(".jsonl"));
  if (!name) throw new Error("missing session log");
  return readFileSync(path.join(sessions, name), "utf8")
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as { kind: string; payload: unknown })
    .filter((entry) => entry.kind === "request_header")
    .map((entry) => entry.payload as Record<string, unknown>);
}

describe("sage -p with a user providers file", () => {
  it("sends --provider example-gateway to the file's base URL and key and logs the file's pricing", async () => {
    const { home, work } = setup(FILE);
    const io = capture();
    const [code, calls] = await withFetch(OPENAI_SSE, async (calls) => [
      await runCli(
        [
          "-p",
          "hi",
          "--provider",
          "example-gateway",
          "--model",
          "example-model",
        ],
        {
          cwd: work,
          env: { HOME: home, EXAMPLE_GATEWAY_KEY: "sk-example" },
          stdout: io.stdout,
          stderr: io.stderr,
        },
      ),
      calls,
    ]);
    expect(io.text().stderr).not.toContain("Error");
    expect(code).toBe(0);
    expect(calls[0]).toEqual({
      url: "https://gateway.example.com/v1/chat/completions",
      authorization: "Bearer sk-example",
      apiKey: null,
    });
    const [header] = requestHeaders(work);
    expect(header).toMatchObject({
      provider: "example-gateway",
      model: "example-model",
      contextWindow: 200_000,
      pricing: CREDIT_PRICING,
    });
    expect(header?.adapterDefaults).toEqual({ maxTokens: 8192 });
  });

  it("lets --base-url and --api-key-env override the file's values", async () => {
    const { home, work } = setup(FILE);
    const io = capture();
    const [code, calls] = await withFetch(OPENAI_SSE, async (calls) => [
      await runCli(
        [
          "-p",
          "hi",
          "--provider",
          "example-gateway",
          "--base-url",
          "https://override.example.com/v2",
          "--api-key-env",
          "EXAMPLE_OTHER_KEY",
        ],
        {
          cwd: work,
          env: {
            HOME: home,
            EXAMPLE_GATEWAY_KEY: "sk-file",
            EXAMPLE_OTHER_KEY: "sk-override",
          },
          stdout: io.stdout,
          stderr: io.stderr,
        },
      ),
      calls,
    ]);
    expect(code).toBe(0);
    expect(calls[0]?.url).toBe(
      "https://override.example.com/v2/chat/completions",
    );
    expect(calls[0]?.authorization).toBe("Bearer sk-override");
    expect(requestHeaders(work)[0]).toMatchObject({
      model: "example-model",
      pricing: CREDIT_PRICING,
    });
  });

  it("speaks the Messages protocol for an anthropic-messages user provider", async () => {
    const { home, work } = setup({
      providers: [{ ...GATEWAY, api: "anthropic-messages" }],
      models: FILE.models,
    });
    const io = capture();
    const [code, calls] = await withFetch(ANTHROPIC_SSE, async (calls) => [
      await runCli(["-p", "hi", "--provider", "example-gateway"], {
        cwd: work,
        env: { HOME: home, EXAMPLE_GATEWAY_KEY: "sk-example" },
        stdout: io.stdout,
        stderr: io.stderr,
      }),
      calls,
    ]);
    expect(code).toBe(0);
    expect(calls[0]).toEqual({
      url: "https://gateway.example.com/v1/v1/messages",
      authorization: null,
      apiKey: "sk-example",
    });
  });

  it("exits 1 and asks for --model when the user provider declares no model", async () => {
    const { home, work } = setup({ providers: [GATEWAY] });
    const io = capture();
    const code = await runCli(["-p", "hi", "--provider", "example-gateway"], {
      cwd: work,
      env: { HOME: home, EXAMPLE_GATEWAY_KEY: "sk-example" },
      stdout: io.stdout,
      stderr: io.stderr,
    });
    expect(code).toBe(1);
    expect(io.text().stderr).toContain("--model");
    expect(io.text().stderr).toContain("example-gateway");
  });

  it("runs as before when the file does not exist", async () => {
    const { home, work } = setup(undefined);
    const { adapter, requests } = createFaux([stopScript("ok")]);
    const io = capture();
    const code = await runCli(["-p", "hi"], {
      cwd: work,
      env: { HOME: home, DEEPSEEK_API_KEY: "sk-deepseek" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { deepseek: adapter as ProviderAdapter },
    });
    expect(code).toBe(0);
    expect(requests[0]?.model).toEqual({
      provider: "deepseek",
      model: "deepseek-flash",
    });
  });

  it("reads no providers file when the CLI env has no HOME, whatever the process HOME holds", async () => {
    const { home, work } = setup("{ providers: [");
    vi.stubEnv("HOME", home);
    const { adapter } = createFaux([stopScript("ok")]);
    const io = capture();
    const code = await runCli(["-p", "hi"], {
      cwd: work,
      env: { DEEPSEEK_API_KEY: "sk-deepseek" },
      stdout: io.stdout,
      stderr: io.stderr,
      adapters: { deepseek: adapter as ProviderAdapter },
    });
    expect(io.text().stderr).not.toContain("providers.json");
    expect(code).toBe(0);
    expect(requestHeaders(work)[0]).toMatchObject({
      provider: "deepseek",
      model: "deepseek-flash",
    });
  });

  const broken: { name: string; file: unknown; field: string }[] = [
    {
      name: "a provider that shadows a built-in preset",
      file: { providers: [{ ...GATEWAY, name: "deepseek" }] },
      field: "providers[0].name",
    },
    {
      name: "a model on an undeclared provider",
      file: { models: FILE.models },
      field: "models[0].provider",
    },
    {
      name: "the same provider and model twice",
      file: { providers: [GATEWAY], models: [...FILE.models, ...FILE.models] },
      field: "models[1].model",
    },
    {
      name: "a price that is not a number",
      file: {
        providers: [GATEWAY],
        models: [
          {
            ...FILE.models[0],
            pricing: { ...CREDIT_PRICING, outputPerMTok: "12" },
          },
        ],
      },
      field: "models[0].pricing.outputPerMTok",
    },
    { name: "a file that is not JSON", file: "{ providers: [", field: "JSON" },
  ];
  for (const row of broken) {
    it(`exits 2 naming the file and field for ${row.name}, before any call`, async () => {
      const { home, work, file } = setup(row.file);
      const { adapter, requests } = createFaux([stopScript("ok")]);
      const io = capture();
      const code = await runCli(["-p", "hi"], {
        cwd: work,
        env: { HOME: home, DEEPSEEK_API_KEY: "sk-deepseek" },
        stdout: io.stdout,
        stderr: io.stderr,
        adapters: { deepseek: adapter as ProviderAdapter },
      });
      expect(code).toBe(2);
      expect(io.text().stdout).toBe("");
      expect(io.text().stderr).toContain(file);
      expect(io.text().stderr).toContain(row.field);
      expect(requests).toHaveLength(0);
    });
  }
});

describe("sage bench run with a user providers file", () => {
  function benchArgv(out: string, tasks: string, extra: string[] = []) {
    return [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "example-model",
      "--out",
      out,
      ...extra,
    ];
  }

  function writeTasks(at: string): string {
    const file = path.join(at, "tasks.json");
    writeFileSync(
      file,
      JSON.stringify([
        {
          id: "one",
          repo: "owner/repo",
          baseCommit: "abc",
          problem: "task",
          testCommand: "true",
          timeoutSec: 30,
        },
      ]),
    );
    return file;
  }

  function recordingDeps(seen: { argv: string[]; env: NodeJS.ProcessEnv }[]) {
    const deps: BenchDeps = {
      async prepare() {},
      async runSage(_task, _cwd, opts) {
        seen.push({ argv: opts.argv, env: opts.env });
        return { code: 0 };
      },
      async diffEmpty() {
        return true;
      },
      async runTests() {
        return { code: 0 };
      },
    };
    return deps;
  }

  it("checks the file's env key and hands each child the same HOME", async () => {
    const { home, work } = setup(FILE);
    const tasks = writeTasks(work);
    const seen: { argv: string[]; env: NodeJS.ProcessEnv }[] = [];
    const io = capture();
    const code = await runCli(
      benchArgv(path.join(work, "out"), tasks, [
        "--provider",
        "example-gateway",
      ]),
      {
        cwd: work,
        env: { HOME: home, EXAMPLE_GATEWAY_KEY: "sk-example" },
        stdout: io.stdout,
        stderr: io.stderr,
        benchDeps: recordingDeps(seen),
      },
    );
    expect(io.text().stderr).not.toContain("providers.json");
    expect(io.text().stderr).not.toContain("Missing");
    expect(code).toBe(0);
    const argv = seen[0]?.argv ?? [];
    expect(argv[argv.indexOf("--provider") + 1]).toBe("example-gateway");
    expect(seen[0]?.env.HOME).toBe(home);

    const missing = capture();
    const missingCode = await runCli(
      benchArgv(path.join(work, "out2"), tasks, [
        "--provider",
        "example-gateway",
      ]),
      {
        cwd: work,
        env: { HOME: home },
        stdout: missing.stdout,
        stderr: missing.stderr,
        benchDeps: recordingDeps([]),
      },
    );
    expect(missingCode).toBe(2);
    expect(missing.text().stderr).toContain("Missing EXAMPLE_GATEWAY_KEY");
  });

  it("exits 1 for a provider that is neither built in nor in the file", async () => {
    const { home, work } = setup(FILE);
    const io = capture();
    const code = await runCli(
      benchArgv(path.join(work, "out"), writeTasks(work), [
        "--provider",
        "example-undeclared",
      ]),
      {
        cwd: work,
        env: { HOME: home, EXAMPLE_GATEWAY_KEY: "sk-example" },
        stdout: io.stdout,
        stderr: io.stderr,
        benchDeps: recordingDeps([]),
      },
    );
    expect(code).toBe(1);
    expect(io.text().stderr).toContain("Unknown provider: example-undeclared");
  });

  it("exits 2 naming the file and field before reading the tasks", async () => {
    const { home, work, file } = setup({
      providers: [{ ...GATEWAY, envKey: "not a name" }],
    });
    const io = capture();
    const code = await runCli(
      benchArgv(path.join(work, "out"), path.join(work, "missing.json")),
      {
        cwd: work,
        env: { HOME: home, ANTHROPIC_API_KEY: "test-key" },
        stdout: io.stdout,
        stderr: io.stderr,
        benchDeps: recordingDeps([]),
      },
    );
    expect(code).toBe(2);
    expect(io.text().stderr).toContain(file);
    expect(io.text().stderr).toContain("providers[0].envKey");
  });
});

describe("route policy with user models", () => {
  it("prices ladder rungs on a user provider from the file", () => {
    const models = [
      {
        provider: "example-gateway",
        model: "example-large",
        contextWindow: 200_000,
        pricing: { ...CREDIT_PRICING, inputPerMTok: 8 },
      },
      {
        provider: "example-gateway",
        model: "example-small",
        contextWindow: 200_000,
        pricing: { ...CREDIT_PRICING, inputPerMTok: 1 },
      },
    ];
    const route = defaultRoutePolicy(
      ["example-gateway"],
      "example-gateway",
      [
        { provider: "example-gateway", model: "example-large" },
        { provider: "example-gateway", model: "example-small" },
      ],
      models,
    );
    const picked = route({
      epoch: 1,
      current: { provider: "example-gateway", model: "example-large" },
      candidates: [],
      viewChunks: [],
      decision: {
        rebuild: true,
        oldTokens: 0,
        newTokens: 0,
        remainingTurns: 1,
        readRate: 0.1,
        writeRate: 1.25,
        savingsPerTurn: 0,
        extraCost: 0,
        paybackTurns: null,
        reason: "test",
      },
      budget: 1000,
    });
    expect(picked?.model).toEqual({
      provider: "example-gateway",
      model: "example-small",
    });
  });
});
