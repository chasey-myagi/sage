# sage

one-shot CLI，发布为 npm 包 `@chasey-myagi/sage`，命令名 `sage`。`sage -p "任务"`（或 `sage "任务"`；在仓库里先 `pnpm build`，再 `node apps/sage/bin/sage.mjs -p "任务"`）在当前目录跑完一轮，把 `RunSummary`（含 `health`）打成一行 JSON 写到 stdout。结果为 `done` 时退出 0，否则退出 1。

```
usage: sage [-p | --print] <task> [--provider <preset>] [--model <id>] [--base-url <url>] [--api-key-env <NAME>] [--ladder provider/model,provider/model] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--tool-sandbox none|full] [--passthrough] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild]
       sage stats [--turns] <session.jsonl>
       sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] [--base-url <url>] [--api-key-env <NAME>] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--tool-sandbox none|full] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild] --out <dir>
       sage bench report [--exclude-faults] <runA> <runB>
       sage calibrate-jev --out <dir> [--max-usd <usd>] [--dry-run] [--t-full <p,p,…>] [--t-low <p,p,…>] [--t-keep <p,p,…>] [--label touch|content] <session.jsonl…>
```

## 参数

任务写在 `-p` / `--print` 之后，或者 `--print=任务`；不写 `-p` 时，所有位置参数用空格连起来就是任务。两种同时出现，或者任务为空，打印 usage 并退出 1。旗标的值可以写成 `--flag value` 或 `--flag=value`；分开写时值不能以 `-` 开头。未知旗标、缺值、`--help` 都打印 usage 并退出 1。同一个旗标写两次，后写的生效。`sage -p` 与 `bench run` 共用一张参数表（`src/options.ts`）和同一套校验与报错文本。

## 自定义 provider：`~/.sage/providers.json`

经 OpenAI / Anthropic 兼容的中转站跑时，可以在用户主目录的 `.sage/providers.json` 声明 provider 和模型单价，不用改仓库。主目录取环境变量 `HOME`，没设时只有内置预设，也不读用户级子任务预设。只读这一个用户级文件，不读工作区里的同名文件：项目目录下的文件容易被提交，会把私有网关的地址和计价公开出去。

```json
{
  "providers": [
    {
      "name": "example-gateway",
      "baseUrl": "https://gateway.example.com/v1",
      "envKey": "EXAMPLE_GATEWAY_KEY",
      "api": "openai-chat"
    }
  ],
  "models": [
    {
      "provider": "example-gateway",
      "model": "example-model",
      "contextWindow": 200000,
      "pricing": {
        "inputPerMTok": 3,
        "outputPerMTok": 12,
        "cacheReadPerMTok": 0.3,
        "cacheWritePerMTok": 3.75,
        "currency": "credit",
        "source": "https://gateway.example.com/pricing"
      }
    }
  ]
}
```

- `api` 是 `openai-chat` 或 `anthropic-messages`。`--provider example-gateway` 选用它，`envKey` 与 `baseUrl` 和内置预设走同一条路径，`--base-url`、`--api-key-env` 照样覆盖。用户 provider 不参与自动选择，要写 `--provider`。没写 `--model` 时取该 provider 在文件里的第一个模型；一个都没声明时退出 1，stderr 提示写 `--model`。
- `models[].provider` 可以是内置预设或本文件声明的 provider。单价按 (provider, model) 查，同一模型 id 在两个 provider 上可以不同价。`currency` 是计价单位的短标签，缺省 USD，中转站自己的单位（例如 `credit`）原样写进 `request_header.pricing`，不换算。`source` 缺省为 `~/.sage/providers.json`。
- 声明过的模型拿到文件里的 `contextWindow` 与单价，`adapterDefaults` 不再带 `catalogFallback`，编译器的阶段重建按这四个单价算读写倍率。回放读 Log 里记录的单价，不读当前文件。
- 文件不存在就只有内置预设。文件存在但读不了、不是 JSON，或者形状不对时退出 2，stderr 一行写出文件路径和出错字段，例如 `models[0].pricing.currency`，不发任何请求。会报错的形状：与内置预设或目录 provider（含 `typesafe`）同名的 provider、重复的 provider 名、引用未声明 provider 的模型、重复的 (provider, model)、与内置目录重复的模型、未知的键、负数或非数字的单价、非正整数的 `contextWindow`。
- `bench run` 先读这个文件再查 key；子进程继承父进程的环境变量，因而是同一个 `HOME`、同一份文件。

## 会话

每次运行装一个 `AgentSession`：七个一方工具，加上 `update_ledger`、`expand`、`spawn_subtask`、`spawn_subtasks`。Log 写在 `<cwd>/.sage/sessions/<id>.jsonl`。子任务预设从 `<cwd>/.sage/subtasks` 和 `$HOME/.sage/subtasks` 读；没设 `HOME` 时只读前者。视图是上下文编译器，带四个一方 claim 和「路由」一节的策略；`--passthrough` 改用 passthrough 视图。子任务总是编译。

`--budget <tokens>` 覆盖默认预算 40000，必须是正整数，否则退出 2，stderr 一行说明。和 `--passthrough` 一起用时忽略这个值，并在 stderr 提示一句。对照实验请显式压预算，默认 40000 在短任务上不会触发编译器。

`--max-turns <n>` 覆盖内核默认的 80 轮（`@sage/core` 的 `DEFAULT_MAX_TURNS`），必须是正整数，否则退出 2，stderr 一行说明。这个数交给 `session.run`，`--passthrough` 同样生效。

## 沙箱

`--tool-network none|full` 默认 `full`。`--tool-sandbox none|full` 是同一个开关，后写的覆盖先写的；值不合法时退出 2，报错里是用户写的那个旗标名。`none` 切断 bash 子进程的网络。在 darwin 上，`none` 还会拒绝读取同一临时根下的其他 `sage-bench-*` 工作区；当前工作目录仍可读，临时目录里的其他文件也可读。父进程照常请求模型。当前平台做不到网络切断时退出 2，stderr 一行说明，不会悄悄放开。linux 的 `unshare -n` 不能限制文件。

环境变量 `SAGE_SANDBOX_ALLOW_ROOT` 设了时，就是 bash 可读的根目录；没设时是当前目录的 realpath。它没有对应的 CLI 旗标，`bench run` 用它把本次运行根目录传给子进程。

## 打分

`--scorer rule|model|jev` 选根编译器的打分器，不写就不装，档位只由淘汰规则决定。`rule` 是缺省档位的规则打分器，不发请求：只有 `bash` 跑测试、构建、脚本这类命令、退出码为 0、且没有接着管道（或开了 `pipefail`）时判 summary，视图里只剩桩行；其余块（`edit`、`write`、`update_ledger` 的一行回显，`read`、`grep`、读取类 bash、报错或接管道的运行类 bash，其他工具）都不改档。分类表与 bash 命令的判定规则见 `packages/plugins/README.md` 的「规则打分器」。`model` 复用已装好的适配器和本次会话的型号。`--passthrough` 不编译上下文，两个打分旗标都不会调用打分器。

`--shadow-scorer rule|model|jev` 给根编译器装上影子打分，默认不装。它每轮只给新进入视图的工具块打分，结果写在清单的 `shadow` 上，`shadow.calls` 是这一轮真正发出的请求数（0 或 1）。影子只记不生效：不改档次、冻结和预算，也不改模型看到的消息。

`--scorer jev` 和 `--shadow-scorer jev` 把打分请求发到 `JEV_BASE_URL`（缺省 `https://api.typesafe.ai`）的 `/v1/systemone`。密钥读 `JEV_API_KEY`，没有则用 `TYPESAFE_API_KEY`。两个都没有时退出 2，stderr 写出这两个变量名。请求体里的 `model` 读 `JEV_MODEL`，缺省 `jev-latest`。接到本地 LLM2Jev（github.com/Yinsongxu/LLM2Jev）时，用它的 `--served-model-name` 作为 `JEV_MODEL`，例如 `JEV_BASE_URL=http://127.0.0.1:30000 JEV_API_KEY=local JEV_MODEL=那个名字 sage -p "任务" --scorer jev`。

`--scorer-apply entry|rebuild` 默认 `rebuild`：打分只在阶段重开时生效。`entry` 改为工具结果进入视图时就定档，前缀从第一次出现就是最终形态。DeepSeek 的缓存是前缀缓存，出窗口时再改档会把后面的近期内容每轮重新全价编码，按写入价和缓存读价要很多轮才回本。`entry` 必须同时写 `--scorer`（`rule`、`model`、`jev` 都可以），否则退出 2，stderr 点名 `--scorer rule|model|jev`。`entry` 不再另跑影子。`runHealth.entryScored` 汇总这次调用的次数、none / summary / full 和 usage。

## `sage stats`

`sage stats <session.jsonl>` 读一个会话文件，不启动运行，打印一行 JSON：`runHealth` 加上影子对账，退出 0。`runHealth` 里的 `scorerCalls`、`entryScored.calls`、`shadow.calls` 都只数真正发出的打分请求，例如 `--shadow-scorer jev` 缺 key 的轮次不计。参数不是单个 `.jsonl` 路径（可另加一个 `--turns`）时打印 usage 并退出 1；文件不存在或读不了时 stderr 一行说明并退出 1。缺 `request_header.contextWindow` 等字段的旧格式会话读不了，同样退出 1。

影子对账是 `shadowNone` 加上 `touch` 与 `content` 两个对象，各为 `{ laterNeeded, precision }`。`shadowNone` 是影子判过 `none` 的块 id 个数，同一个 id 只算一次。`laterNeeded` 是其中按该标签（定义见下面的「后续使用标签」）后来仍需要的个数，判定点是第一次判 `none` 的那条清单。`precision = 1 − laterNeeded / shadowNone`，`shadowNone` 为 0 时是 null。这些数是事后标签，编译器不读它们。

### 后续使用标签

`sage stats` 的影子对账和 `sage calibrate-jev` 的扫描用同一份代码（`src/later-use.ts`）给每个工具块算两个标签，判定点之后的事才算：

- **touch（路径再碰）。**块 id 出现在之后某次 `expand` 的参数 `id` 里；或者块是 `read` 的结果，同一 `path` 在判定那一轮之后的轮次又被 `read` 或 `edit`。判定那一轮的回复里紧接着重读或编辑不算。
- **content（内容被用到）。**块之后被 `expand`；或者之后某次调用用到的字符串由这块输出提供（规则见下）；或者块是 `bash`，之后又跑了规范化后相同的命令（`normalizeBashCommand`，见 `packages/plugins/README.md`）。

调用用到的字符串分两类，各按一条规则找「出处」：

- **抄写的原文：`edit` 的 `old_string`，按最近出处。**取调用之前最近的一处副本。原文是照着最近一次看到的文件抄的：文件重读过，功劳给重读的那块；同一行先在 grep 命中里出现、之后又被 read 读到，功劳给 read。
- **名字：路径与搜索模式，按首次出处。**取全文里第一次出现的位置，并且这个位置要在调用之前。名字一旦出现就是已知的，之后的列表或重读再列出它不算提供了它；任务里已经写过的名字不记给任何块。包括：grep 工具的 `pattern`；bash 命令里 `grep` / `rg` 各级的模式（`bashSearchPatterns`）；`read`、`edit`、`write` 的 `path`；bash 读取类各级（`cat`、`head`、`sed -n` 等）打开的文件（`bashFileOperands`）。

两条规则共用的部分：

- 全文是把路径上所有消息的文字按 Log 顺序拼起来：用户与 assistant 的文字、thinking、工具输入里的每个字符串值、工具结果。出处落在某块的工具结果里，功劳就记给那块；落在用户或 assistant 的文字里、或者某次工具输入里，就不记给任何块。块自己输入里的路径也不算它提供的。
- 只看发出那次调用的 assistant 消息之前的文字。少于 3 个字符的字符串不计。
- 正则模式先把标点前的反斜杠去掉（`foo\(` 按 `foo(` 找），其余元字符照原样，按字面子串匹配。
- 路径依次按三种写法查，哪种在调用之前有副本就用哪种：先换成相对工作区根目录的路径（根目录取 `request_header.system` 最后一行 `Workspace: <绝对路径>`，工具输出里的路径都相对它）；再按原样（去掉开头的 `./`）；最后按至少两段的后缀，从长到短（`/x/repo/src/a.ts` 依次试 `repo/src/a.ts`、`src/a.ts`）。

算法是一次扫描：拼出全文并记下每段的起点和来源，再逐个调用查它的字符串。名字做一次 `indexOf`，按写法缓存，因为首次出处与哪次调用无关；`old_string` 从调用位置向前做一次 `lastIndexOf`，碰到最近的副本就停。再二分查找副本落在哪一段。复杂度是 O(N + F·N)，N 是全文字符数，F 是要查的写法数（名字按不同写法计、`old_string` 按调用计）。没有变换过的字符串一定出现在发出它的调用的输入里，所以 `indexOf` 最晚停在那里；`lastIndexOf` 通常在最近读过的那块就停。重跑命令按规范化后的命令分组，查一次是 O(同一命令的次数)。

两个标签的已知偏差：

- 块越大、判定之后剩下的轮数越多，越容易命中，这对两个标签都成立。
- touch 把「改文件前照例重读」也算作需要，read 的假阳性多；它几乎从不标 grep 和 bash，只能靠 `expand`。
- content 只认上面这些字符串和重跑。模型读了内容后凭理解写出的新代码、换了写法的模式、不在工作区下也凑不出两段后缀的路径，都认不出。
- 反过来，短而常见的名字只要首次出现在某块里就记给它。两段后缀可能撞上别的目录下的同名文件。
- 系统提示不进全文，系统提示里写过的路径会记给第一次输出它的块。
- bash 读取类的文件按写法匹配，前面 `cd` 到子目录时得到的是相对子目录的路径。
- 同一条命令跑了几次，除最后一次外每次都记为用到。
- `old_string` 只看调用之前最近的一份副本：最近一份落在 thinking 或上一次 edit 的输入里时谁都不记，哪怕更早有一份在工具结果里。
- 经 stdin 重定向读文件（`wc -l < src/a.ts`、`sort < f`）不算打开；`grep -n x a.ts`、`git diff a.ts` 这类非读取类命令里的文件也不算。


### `--turns`

`sage stats --turns <session.jsonl>`（`--turns` 写在文件前后都行）改为逐轮审计，同样打印一行 JSON、同样的退出码：`{ turns, invariants }`。用 `jq -c '.turns[]'` 可以拆成每轮一行。

一轮是一条 `request_header` 加上它之后的第一条 assistant 回复。没有回复的请求（中止或失败的调用）不出行。`turns` 里每项：

| 字段 | 口径 |
|---|---|
| `turn` | 该轮清单的 `turn`；没有清单时是行序号（从 1 起） |
| `epoch` | 该轮清单的 `epoch`，没有清单时为 null |
| `model` | 该轮 `request_header` 的 `{ provider, model }` |
| `tokens` | 该轮 assistant usage，字段同 `runHealth.tokens`（`input` 只算未命中缓存的输入） |
| `cacheHitRatio` | 只按这一轮算的 `runHealth.cacheHitRatio`，分母为 0 或没有 usage 时为 null |
| `budget` | 清单 `budget` 的 `{ used, omitted, expands }`；`budget` 不是对象时为 null。`expands` 是截至本轮的累计数 |
| `tiers` | 清单块数按档位计的 `{ full, summary, line, none }` |
| `flips` | 与上一条带清单的轮次同属一个 epoch 时，同一块 id 档位变了的 `{ id, from, to }`；跨 epoch 不算翻档，为空数组 |
| `epochDecision` | 清单 `epochDecision` 的 `{ rebuild, oldTokens, newTokens, remainingTurns, paybackTurns }`，没有时为 null。`paybackTurns` 缺失或没有节省时为 null。不输出 `reason`：路由策略会把命中的敏感文件路径拼进去 |
| `tools` | 该轮回复里 tool_use 的工具名，按出现顺序 |

`invariants` 汇总整条路径：`toolUses`、`toolResults` 是两种块的个数；`unpairedToolUses` 是路径上找不到 tool_result 的 tool_use id，`orphanToolResults` 是找不到 tool_use 的 tool_result id；`toolErrors` 是 `isError: true` 的结果数；`requestHeaders`、`manifests` 是两种条目的条数；`models` 是出现过的 `{ provider, model }`，按首次出现排序；`outcome` 是最后一条 `terminal` 的 `outcome`，没有时为 null；`health` 是这条路径的 `runHealth`。

输出只有计数、块 id、tool_use id、工具名和档位，不含消息文本、任何 `reason` 字符串、工具输入和路径。

## `sage calibrate-jev`

`sage calibrate-jev <session.jsonl…> --out <dir>` 用录好的会话离线校准 jev 打分器的阈值。只调 Jev，不调主模型，也不改会话文件。

**请求。**每条会话按轮回放：每个工具块在第一次进入视图的那一轮，若不是 `expand` 结果、估算不少于 300 token（入口打分的 `minScoreTokens` 默认值），就按入口模式的方式构造一个 Jev 请求：块本身、原文、发出这次调用的 assistant 文字、该轮最新的用户问题，以及从 Log 重建的该轮请求里的进度账本。问题集与线上打分器同样按块类型选：grep 与运行类、混合类 bash 问分诊问题集，请求体与线上入口打分发出的逐字节相同；read 问试验性的 `shows_target_code` 探针，线上不发；其余块（含读取类 bash）计入 `skipped.unasked`。bash 的分类与线上同用 `classifyBashCommand`。与线上不同的还有不要求该块此时仍是 full：被预算裁掉的块也会问。工具已标 `isError` 的块线上不发问、直接判 full，这里同样不发，计入 `skipped.isError`；没有原文或塞不进 Jev 长度上限的块计入 `skipped.unsendable`。passthrough 会话没有申领单，请求里的 `ledger` 恒为空串，Jev 看到的输入分布与 compiler 会话不同。地址、密钥和 `model` 的读取与 `--scorer jev` 相同（`JEV_BASE_URL`、`JEV_API_KEY` / `TYPESAFE_API_KEY`、`JEV_MODEL`）。

**发送。**与打分器共用同一个发送池：同时最多 4 个请求在途，429 与 529 按 `retry-after` 退避，单次超时 30 秒。单个请求失败（其他非 2xx、超时、响应里没有 `answers`）只计入 `failed`，其余请求照发；某个请求的 429 / 529 重试用尽时熔断，尚未发出的请求不再发，计入 `unsent`，`stopped` 为 `{ kind: "rate-limited", fallback }`。有失败或熔断时，结果照常写出，退出 1。

**缓存。**每个回答存到 `<out>/cache/<请求体的 sha256>.json`。重跑时命中缓存的请求不发、不计费；换了 `JEV_MODEL` 或会话内容变了，请求体不同，就是新请求。几条会话问出逐字节相同的请求时只发一次。

**预算。**发请求前先估算未命中缓存的请求的输入 token：每个请求是 state 加全部问题的 UTF-8 字节数除以 2.5 再向上取整，与打分器控制请求长度用的是同一个估算；Jev 按 state 计一次加各问题计费，实测每 token 约 2.4–3 字节，所以估算与实际接近，可能偏少几个百分点，真正封顶靠执行中的检查。美元数是 `token × 0.042 / 1e6`，单价取自内置目录 `typesafe/jev-latest` 一行（只计输入，输出不计费）。`--max-usd`（默认 1，可写 0 或小数）是跨次的累计上限：`<out>/spend.json` 里已有的花费加上本次估算超过它时，不发任何请求，stderr 一行说明（含已花的累计数）并退出 2。`--dry-run` 只往 stdout 打一行估算 JSON（`dryRun`、`sessions`、`chunks`、`cached`、`pending`、`estimate { inputTokens, usd }`、`priorUsd`、`maxUsd`、`withinBudget`）并退出 0，不发请求、不写文件，也不需要密钥。实际用量取响应里的 `usage.input_tokens`，每答完一个请求就把跨次累计的 `{ requests, inputTokens, outputTokens, usd }` 写进 `<out>/spend.json`。累计实际花费一旦超过 `--max-usd`，不再发新请求，在途的照常收完，已拿到的结果照常写出，退出 2。每个请求开始前才检查上限，越线时最多还有 4 个请求在途，所以超支不超过 4 个请求的费用；单个请求受 Jev 的 64k token 上限约束，上界约为 `4 × 64000 × 0.042 / 1e6 ≈ $0.011`。

**标签。**每个块同时算 touch 与 content 两个标签（定义与偏差见上面的「后续使用标签」），判定点是该块进入视图那一轮的视图提交起点。扫描按 `--label touch|content` 选的标签算「需要」，默认 `content`；其他值打印 usage 并退出 1。轮次按清单的 `turn` 算，compiler 与 passthrough 会话都会写清单。passthrough 从不省略，模型没有理由 `expand`，touch 只能靠重读和编辑打上，grep 与 bash 块在 touch 下几乎全是「不需要」；content 不依赖 `expand`，两种会话都能标出它们。

**扫描。**对 `--t-full`、`--t-low`、`--t-keep`（逗号分隔、取值 [0, 1]）的每种组合，用打分器自己的定档函数把缓存的分诊回答重新定档；read 探针不定档，不进扫描。默认网格是 full `0.2,0.3,0.4,0.5,0.6`、low `0.1,0.2,0.3`、keep `0.3,0.5,0.7`，共 45 组。summary 和 none 都算「省略」：两者都把原文移出视图。每组输出：

| 字段 | 口径 |
|---|---|
| `thresholds` | 这一组的 `{ full, low, keep }` |
| `tiers` | `{ full, summary, none }` 各档块数 |
| `omitted` / `needed` / `omittedNeeded` | 省略的块数、后来需要的块数、两者都是的块数 |
| `missRate` | 漏判率 `omittedNeeded ÷ needed`；没有块需要时为 null |
| `precision` | 省略且从未需要的块数 ÷ 省略的块数；没有省略时为 null |
| `recall` | 省略且从未需要的块数 ÷ 从未需要的块数；所有块都需要时为 null |
| `cost` | 期望成本，单位是「输入价等价 token」，费率与编译器的阶段决策同一个 `ratesFor`：`read = cacheRead / input`，`write = max(cacheWrite, input) / input`，按该块进入那一轮的 `request_header.pricing` 算，没有定价时用默认的 0.1 与 1.25。省略后又被需要的块付一次取回：`tokens × write`；保留 full 却从未用到的块付 `tokens × read × 剩余轮数`。不同币种、没有内置定价的会话可以直接相加 |

`cost` 没有计入三项：expand 多出的那一轮模型调用；取回的文本在之后各轮的缓存读；取回时实际的写价差异（取回按写价计，实际可能部分命中缓存）。summary 笔记本身占的 token 也不计。

结果写入 `<out>/sweep.json`：`{ grid, label, chunks, rows, recommended }`，`label` 是扫描用的标签，`rows` 里的 `needed` 与 `omittedNeeded` 都按它算。`chunks` 列出每个拿到回答的块，含 read 探针，每项是 `{ session, chunkId, tool, bashKind, turn, ledgerEmpty, tokens, remainingTurns, touch, content, questions, answers }`，`bashKind` 是 bash 块的 `classifyBashCommand` 结果（`reader` / `runner` / `mixed`），其他块为 null，`turn` 是块进入视图的那一轮，`ledgerEmpty` 是重建的请求里 `ledger` 是否为空串（passthrough 会话恒为 true，compiler 会话在第一次 `update_ledger` 之前也是 true），`touch` 与 `content` 是两个标签，`session` 是会话文件名（块 id 跨会话不唯一），`questions` 是问题集（`triage` 或 `read-probe`），`answers` 是各问题的原始概率：分诊块是 `{ reports_failure, locates_next_step, keep }`（`keep` 按候选行顺序排的数组），read 探针是 `{ shows_target_code }`。推荐取 `cost` 最低的一组；相同时取离 `DEFAULT_JEV_THRESHOLDS` 最近（三个值的欧氏距离）的一组，再相同取网格里靠前的。stdout 打一行汇总 JSON：`sessions`、`chunks`、`cached`、`sent`（本次发出的请求，含失败的）、`failed`、`unsent`、`skipped { isError, unasked, unsendable, unanswered }`、`unpriced`（进扫描的块里没有定价、按默认费率计的块数）、`estimate`、`spent`（本次）、`totalSpentUsd`（累计）、`stopped`（null、`{ kind: "budget" }` 或 `{ kind: "rate-limited", fallback }`）、`label`、`recommended`。`unanswered` 是没有回答或回答里缺该问题集固定判断的块。

`sweep.json`、`spend.json` 和 stdout 只有会话文件名、块 id、工具名、计数、概率和价格，不含会话文本；缓存文件只存 Jev 的 `answers` 和用量。参数错误打印 usage 并退出 1；`--max-usd` 或阈值写错退出 2；会话文件不存在、需要发请求却没有密钥时 stderr 一行说明并退出 1。

## `sage bench`

离线跑分，不是 CI 闸门。

`bench run` 先查 key：选哪家由 `--provider` 决定，没写时按自动选择顺序取第一个设了预设 key 的那家；`--api-key-env` 只替换读哪个变量，不参与选家。选中那家要读的变量没有值时报错，文本和 `sage -p` 相同。用户文件里声明的 provider 也能用 `--provider` 选，见「自定义 provider」。`--base-url` 与 `--api-key-env` 写了就原样转发给每个任务的 `sage -p`，没写不传；含义与 `sage -p` 相同。`~/.sage/providers.json` 坏了或缺 key 都退出 2，此时还没读任务文件、没拉仓库。`--provider` 既不是内置预设也不在用户文件里时，stderr 写 `Unknown provider: <name>` 并退出 1。

任务文件是 JSON 数组，每项 `{ id, repo, baseCommit, problem, testCommand, timeoutSec }`，可选 `setupCommand` 与 `testPatch`。任何一项不合法都在开跑前报错并退出 1。

每个任务依次执行，第一处失败决定结果：

1. 拉取到 `<realpath(tmpdir)>/sage-bench-<随机>/<taskId>/`：`git init`，加 origin，`git fetch --depth 1 --no-tags` 拉 `baseCommit`，`git checkout -q FETCH_HEAD`，再删掉 origin。`repo` 不带 scheme 时拉 `https://github.com/<repo>.git`。拉取失败是 `clone-failed`。拉完必须恰好一个提交、没有 tag、没有 remote，否则是 `clone-unclean`，setup 和 agent 都不跑。
2. 有 `setupCommand` 时在该目录用 `sh -c` 跑，受 `timeoutSec` 限制。非零退出是 `setup-failed`：不启动 sage，也不跑测试。
3. 在该目录跑 `sage -p <problem>`。argv 总带 `--max-turns`（写了旗标用那个数，没写就是 80）和 `--tool-network`；`--budget`、`--scorer`、`--shadow-scorer`、`--scorer-apply` 只在写了时转发，passthrough 下也转发。启动失败是 `sage-failed`，超时是 `timeout`。随后把会话 jsonl 复制到 `<out>/sessions/<taskId>.jsonl`。
4. 会话最后一条 `terminal` 的 `outcome` 是 `error` 时记 `session-error`，是 `aborted` 时记 `session-aborted`：不看 diff、不跑测试，工作区有改动也一样。`max_turns` 与 `done` 照常往下判。`session-error` 的结果行带 `providerError`：最后一条助手消息以 `finish.kind === "error"` 结束时按 `finish.error.cause` 取四种形状之一：`{ cause: "transport" }`；`{ cause: "http", status }`，`status` 是 HTTP 状态码；`{ cause: "event", event }`，`event` 是 `finish.error.event` 的 `{ transient, type, code, status }`，缺的字段为 null，这里的 `status` 是事件自带的字段而不是 HTTP 状态；`{ cause: "other" }`，Log 里的 `cause` 缺失、不认识，或该 cause 的专属字段坏了，也记成这一种。会话因 provider 以外的原因出错（seam 断言、没有对应 adapter）时为 null。错误文本不写进结果。会话读不到时这一步跳过。
5. `git status` 为空是 `empty-patch`（sage 自己的运行时目录 `.sage/` 不算改动，例如会话 Log 与溢出的工具输出），不跑测试；读不到 status 是 `diff-failed`。
6. 有 `testPatch` 时，先把 diff 头里每个路径从 `baseCommit` 恢复（`git checkout`，基线里没有的文件就删掉），再 `git apply`。失败是 `test-patch-failed`，不跑测试。agent 看不到 `testPatch`。
7. 跑 `testCommand`：退出 0 是 `resolved`，超时是 `timeout`，其他是 `tests-failed`。

`health` 是复制出来的会话的 `runHealth`，读不到时为 null。

`stoppedEarly` 是工具顺序的启发式指标：会话以 `done` 结束、最后一次成功的 `edit` / `write` 之后没有 `bash` 结果，且工作区 diff 非空时为 true。失败的编辑忽略，失败的 bash 也算一次检查尝试。不解析命令或输出，因此不能证明是否执行了测试，也不识别 bash 内的文件修改。只读父会话：子任务会话里的 `bash` / `edit` 不计入，所以父会话 `edit` 之后交给子任务跑测试，也会被记为早停（误报）。空 diff 且会话可读时为 false；会话不可读、diff 读取失败、子进程超时或会话以 `error` / `aborted` 结束而未检查 diff 时为 null。它只影响统计，不改变 agent 行为或任务状态。

结果文件同时保存 `stoppedEarly`。报告的 `stoppedEarlyRate` 是 true 的任务数除以全部任务数（含未知项），空任务集为 null；差值仍为 B 减 A。

`<out>/results.json` 每跑完一个任务重写一次，只存 id、状态、原因、`session-error` 行的 `providerError`、health 数字和 stoppedEarly，不复制题面。顶层字段：`budget` 是 compiler 策略下 `--budget` 的值，没写或 passthrough 时为 null；`toolNetwork` 默认 `none`，写 `--tool-network full` 才是 `full`；`maxTurns` 是 `--max-turns` 的值或 80，两种策略都记；`scorerApply` 是 `entry` 或 `rebuild`，没写记 `rebuild`。`provider` 是 `--provider` 的值，没写时是按自动选择顺序选中的那家（子进程拿到同一份环境变量，选的是同一家）；`baseUrl` 是 `--base-url` 的值，`apiKeyEnv` 是 `--api-key-env` 的变量名，没写时都为 null；key 的值不写进这份文件。

bench 的网络默认 `none`，并总是写进子进程的 `--tool-network`。这时每个子进程的 bash 只能读本次运行根目录，任务可以 `cd` 到自己的绝对路径，其他 `sage-bench-*` 根目录仍被拒绝；运行根目录经 `SAGE_SANDBOX_ALLOW_ROOT` 传给子进程。开跑前，临时根下遗留的 `sage-bench-*` 目录会在 stderr 列出并提示手动清理，不会被删除。每个任务目录在任务结束时删除，运行根目录在整次运行结束时删除。全部任务跑完退出 0。

`bench report [--exclude-faults] <runA> <runB>` 打印一个 JSON 对象 `{ a, b, delta, excluded }` 并退出 0。位置参数不是两个目录、或写了 `--exclude-faults` 以外的旗标时打印 usage 并退出 1，缺 `results.json` 时退出 1；`--exclude-faults` 写在目录前后都可以。`a`、`b` 各带该次的 `budget`、`baseUrl` 与 `provider`；`results.json` 里没有 `provider` 或 `baseUrl` 时为 null。比率都以参与比较的任务数为分母。

故障格是与 agent 无关、因基础设施或上游丢掉的任务：`status` 为 `error` 且 `reason` 是 `clone-failed`、`clone-unclean`、`setup-failed`、`sage-failed`、`session-error`、`session-aborted`、`diff-failed` 之一。`timeout`、`test-patch-failed`（agent 自己的改动可能让测试补丁打不上）和 `tests-failed` 算 agent 的结果，不是故障；其他 `reason` 字符串也不算。分类只在 `src/bench/fault.ts` 定义一处。`faultRate` 是故障格数除以任务数；`errorRate` 含故障格，`errorRate - faultRate` 是 agent 自己失败的占比。故障格不计入 `emptyPatchRate` 和 `resolvedRate`。

不写 `--exclude-faults` 时每个任务都参与比较，`excluded` 为空数组。写了之后按任务 id 对称剔除：任一侧是故障格的任务，两侧都去掉再算全部比率，`excluded` 列出被剔除的 id，每个只出现一次，先按 A 的顺序、再补 B 独有的。剔除后两侧的 `faultRate` 都是 0。`tokensPerResolved` 是 `health.tokens.total` 之和除以 resolved 数，没有 resolved 时为 null。`cacheHitRatio` 和 `expandRate` 是各任务非 null 值的均值。`scorerFailedRequests` 不是比率而是计数：各任务 health 里 `scorerFailedRequests`、`entryScored.failedRequests`、`shadow.failedRequests` 的总和，即同一次打分里其他请求答上了、自己却失败的打分请求数；没有任何任务带 health 时为 null。`delta` 是 B 减 A，任一侧为 null 时该项为 null。

## Model Experience

### What the model sees

argv 和 stderr 进度不是模型可见面。本壳选定的 system prompt 和十一个工具 schema（七个一方工具，加上 `update_ledger`、`expand`、`spawn_subtask`、`spawn_subtasks`）会进入请求。两者都按 hash 写入 `request_header`：首见或与上一条不同时才存全文。进度行只在 stderr：`turn N · tokens in/out · tool calls`。

### Token effect

system prompt 是固定的几行；system prompt 与工具 schema 不变时，后续 `request_header` 只存两个 hash。工具页的长度由 `@sage/tools` 的双限位和 spill 卡住。

### KV cache effect

同一次运行里 system prompt 与工具 schema 不改，head 前缀保持稳定。工具结果接在前缀之后。

## 路由

默认梯子只有当前型号。多家 key 都在时，阶段重开不会改跑更便宜的型号。要跨型号，显式传 `--ladder provider/model,provider/model`，在这些档里选目录 input 价最低、且这次视图允许的那一档。目录包括 `~/.sage/providers.json` 里声明的模型；用户 provider 的 key 设了才有适配器，才会出现在候选里。新视图碰到 `.env` 或 `secrets` 时，非当前 provider 的档会被拿掉。`--help` 只列出 `--ladder` 的写法，不含这段说明。

## `--provider`

`--provider <name>` 选用预设。`--base-url` 覆盖该预设的基址，`--api-key-env` 覆盖读哪一个环境变量。没拿到 key 时退出 2，stderr 点名这个变量。不认识的预设退出 1。`--provider typesafe` 被拒绝并退出 1：目录里那一行是给 Jev 打分计价的，不是对话端点。没写 `--provider` 时，按 deepseek、dashscope、moonshot、openai、anthropic 取第一个已经设了 key 的预设；一个都没有时退出 1。显式 `--provider` 盖住这个顺序。不写 `--model` 时用该 provider 目录的第一条：deepseek 是 `deepseek-flash`，`deepseek-v4-pro` 用 `--model` 选。每个已经设了 key 的预设都会装上适配器：OpenAI 兼容的几家是同一个适配器类，基址不同。下表是内置预设；`~/.sage/providers.json` 里声明的 provider 走同一条路径，但不参与自动选择，见「自定义 provider」。

| provider | base URL | 环境变量 | api |
| --- | --- | --- | --- |
| openai | `https://api.openai.com/v1` | `OPENAI_API_KEY` | openai-chat |
| deepseek | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` | openai-chat |
| dashscope | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` | openai-chat |
| moonshot | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` | openai-chat |
| anthropic | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | anthropic-messages |
