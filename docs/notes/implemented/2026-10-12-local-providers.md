# Agent Note: 用户文件声明 provider 与模型单价

## Problem

provider 预设与模型目录只在 `@sage/provider` 的源码里。经 OpenAI / Anthropic 兼容中转站调用时，`--base-url` + `--api-key-env` 能把请求送过去，但目录外的模型 `pricing: null`、`adapterDefaults.catalogFallback: true`：编译器的阶段重建只能用兜底的 0.1 / 1.25 读写倍率，`request_header` 里也没有单价可供事后算费用。中转站常按自己的单位（例如 credit）计费，而 `Pricing.currency` 只收 `USD` / `CNY`。加一个模型只能改仓库，私有网关信息又不该进公开仓库。

## Decision

- **文件只有用户级**：`<HOME>/.sage/providers.json`，目录名用 core 的 `SAGE_DIR_NAME`。HOME 取 CLI 收到的 `env.HOME`，没有时用 `os.homedir()`；生产环境里两者相同，而测试可以经 `env` 指向临时目录，bench 子进程拿到的是同一份 env，所以读同一个文件。不读工作区里的同名文件：项目文件容易被提交，把私有网关泄露出去。
- **IO 在 apps，校验在 provider**：`apps/sage/src/local-providers.ts` 读文件、`JSON.parse`，失败以值返回，带文件路径与字段的一行 stderr 文本；`sage -p` 与 `bench run` 共用这一个入口 `loadLocalProviders(env)`。`@sage/provider` 的 `parseLocalProviders(unknown)` 是纯函数，返回 `{ ok, value } | { ok: false, field, message }`；它必须知道内置预设和目录才能判断冲突，所以放在 provider 包而不是 apps。provider 包不碰 fs。
- **配置错误即退出**：文件不存在等于没有自定义；文件读不了、不是 JSON、形状不对，`sage -p` 与 `bench run` 都退出 2，stderr 一行「路径: 字段: 原因」。冲突规则：用户 provider 与内置预设或目录 provider（含 `typesafe`）同名、重名；模型引用未声明的 provider；(provider, model) 重复或与内置目录重复；未知键。冲突一律报错，而不是让用户条目覆盖内置条目——同一个 (provider, model) 只有一个定义。
- **查价键**：目录本来就按 (provider, model) 查（`prepareModel` 比较两个字段），不需要改键。改的是数据来源：`listModels` / `defaultModel` / `prepareModel` / `resolvePreset` 接受可选的用户条目，接在内置表之后；适配器构造参数新增 `models`，`prepare` 用它。`defaultModel` 在没有条目时返回 `undefined`，不再抛错，`resolveAuth` 据此退出 1 并提示 `--model`。
- **计价单位**：`Pricing.currency` 从 `"USD" | "CNY"` 放宽成短标签字符串（`PRICE_UNIT`：字母开头，最多 16 个字母、数字、`_`、`-`），缺省仍是 USD，内置条目不变。缓存倍率本来就只除四个单价，不看这个字段；`request_header.pricing.currency` 原样带出。
- **L4**：用户单价经 `PreparedCall.pricing` 写进每条 `request_header`（`commit.ts` 深拷贝），离线回放读的是记录的值，不读当前文件。文件本身不进 Log。
- **路由**：`adaptersFromEnv` 也为设了 key 的用户 provider 建适配器；`defaultRoutePolicy` 的候选和 `costTierPolicy` 的查价都接上用户条目（`costTierPolicy` 新增 `localModels` 选项，仍然不信任候选条目自带的价格）。用户 provider 不参与自动选择。
- **bench**：`parseRun` 不再在解析期查预设（解析期还没有用户文件），未知 provider 改由 `resolveBenchProvider` 报 `Unknown provider: <name>`，退出码仍是 1；缺 key 仍是 2。`resolveAuth` 与 `resolveBenchProvider` 共用 `selectPreset`，失败带判别字段 `kind`，两边各自映射退出码（`-p` 无 key 时 1，bench 无 key 时 2，与改动前一致）。

## Alternatives considered

- **新增 `unit` 字段、保留 `currency: "USD" | "CNY"`**：同一个事实（价格的单位）会有两个来源，消费者要判断读哪一个。不选。
- **把 `currency` 改名为 `unit`**：语义更准，但要改 Log 字段名、所有内置条目和测试，收益只是名字。字段的 JSDoc 已写明它是「计价单位」，不选。
- **用户条目覆盖内置条目**（例如同一个 `anthropic/claude-sonnet-5` 写不同价）：中转站和官方端点是两个 provider，应该声明成两个名字；允许覆盖会让同一个 (provider, model) 有两个定义，读 Log 的人分不清用的是哪份价。不选。
- **项目级 `.sage/providers.json`**：会被提交进仓库。不做。
- **provider 包自己读文件**：分层规则允许 provider 用 node 内置模块，但 provider 包现在没有任何 fs 代码，HOME 的解析和退出码属于 CLI。不选。
- **在 `@sage/core/testing` 的 setup 里把 HOME 指到临时目录**，让没有显式传 `HOME` 的测试也不读开发者的真文件：bench 相关测试会调用 git，改 HOME 会影响 git 的全局配置读取。没做；新测试都显式传 `env.HOME`。合法的用户文件只追加非内置条目，不改变既有测试看到的内置行为；坏的用户文件会让依赖真实 HOME 的测试以退出 2 失败，这是有意的可见失败。

## Consequences

- `Pricing.currency` 接受任意符合 `PRICE_UNIT` 的标签，`EUR` 之类此前会被 `isPricing` 拒绝的值现在合法；`packages/core/test/log/registry.test.ts` 里代表「坏单位」的样本因此从 `EUR` 改为含空格的 `not a unit`。
- `sage bench run --provider <未知名>` 的 stderr 从 usage 文本变成 `Unknown provider: <name>`，退出码不变。
- 新增运行时依赖：无。

## Testing

- `packages/provider/test/local-providers.test.ts`：合法文件的解析结果、`source` 缺省值；21 种坏形状各自报出的字段路径；同一模型 id 在两个用户 provider 上拿到各自的窗口与单价；`isPricing` 接受 `credit`、拒绝空串与含空格的标签。
- `apps/sage/test/local-providers.test.ts`（经 `runCli`）：`--provider example-gateway` 的请求 URL 与鉴权头、`request_header.pricing` / `contextWindow` / `adapterDefaults`；`--base-url` 与 `--api-key-env` 覆盖文件值；`anthropic-messages` 走 Messages 协议；未声明模型时退出 1；文件不存在时行为不变；五种坏文件退出 2 且 stderr 含路径与字段、没有发出请求；bench 预检按文件的 envKey 查 key，子进程 env 的 HOME 与父进程相同，未知 provider 退出 1，坏文件在读任务文件前退出 2；用户模型在 ladder 上按文件单价选档。
- `packages/plugins/test/routing/policy.test.ts`：`costTierPolicy` 只有在传 `localModels` 时才给用户条目定价。
- `pnpm check`。
