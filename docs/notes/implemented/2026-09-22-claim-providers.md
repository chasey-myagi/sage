# 四个第一方申领单

## Problem

系统提醒、AGENTS.md、工具目录和进度账本如果写进 system prompt，每一轮改一个字都会让前缀缓存失效。它们也还没有档位和落点，编译器无法在总预算下决定这一轮给模型看多少。

## Decision

四个第一方 provider 只提交申领单。系统提醒是 tail、priority 20、只有 line 档。工作区 `AGENTS.md` 是 epoch-head、priority 80，full 为全文，summary 为前 40 行，超过则再加一行 `…`。子目录 `AGENTS.md` 只在某次 assistant `tool_use.input` 的字符串字段 `path` 落在该目录之下时出现，id 为 `agents-md:<dir>`，placement `tail-if-changed`，priority 70。工具目录是 epoch-head、priority 40，line 为每个延迟工具一行 `- name — description`，full 在这组行之后附上 inputSchema JSON；列表为空则不产出。进度账本是 pinned、`tail-if-changed`、priority 100、只有 full 档。`update_ledger` 的 execute 只返回 `Ledger updated.`，正文从 Log 里的 tool_use input 折叠，后者覆盖前者，数组字段整体替换。外部文本里的 `</claim` 换成 `<\/claim`。

`createCodingSession` 默认用 `contextCompiler({ budget: 40000 })` 装上这四张单，并把 `update_ledger` 加进工具列表。`--passthrough` 切回 `passthroughView`。

## Alternatives considered

账本放在 epoch-head。进度一轮里可能只改几个字。放进阶段头，一变就要重开阶段，前缀缓存整段作废。挂在 `tail-if-changed`，没变就不写新全文，变了也只动消息末尾。

账本工具做校验，或在结果里催模型更新。校验会把「记下任务框架」变成一道闸；催促和驱动是这份设计明确不做的。工具不解释输入，也不追加下一轮指令。框架只靠钉住的申领单留在视图里。

工具目录在呈现的同时改 `head.tools`，把延迟工具激活进本阶段。激活会改 tools 块的哈希，阶段内前缀缓存失效。目录只占一行或一份 schema，激活留到阶段边界。

## Consequences

没有 `update_ledger` 调用时不产出账本。调用了但没有任何可打印的段，同样不产出。子目录规则只看 path 的祖先目录里有没有 `AGENTS.md`，不扫描整棵树，也不做任务类型命中。

summary 档是前 40 行，超长加 `…`，不附加「这是你自己的笔记，不是证据」前言，也不改成头尾摘录。这是本票钉死的档位正文。E3 的前言若要加，会改变 summary 的字节，需要另开一票。

CLI 默认预算 40000。`toolCatalog` 的 deferred 为空，所以默认会话没有工具目录。直通对照必须显式 `--passthrough`，否则 manifest 带 budget，不再是 passthrough 清单。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、147 个测试）。红灯时四个 provider 模块不存在（`Cannot find module '../../src/claims/index.ts'`）；端到端第一轮 `messages[0]` 仍是用户任务 `rename greet`，不含 AGENTS.md。
