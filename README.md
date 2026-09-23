# sage

一个编码 agent 的运行时。它的特色不是「又一个能跑 coding agent 的 harness」，而是**上下文是被编译出来、可审计、跨 provider 的产物**。

## 三样别人做不到的

1. **看得见每个 token 为什么在那里**——每次 LLM call 留一份编译清单，写进 log。
2. **阶段边界零代价换模型**——前缀按阶段冻结，边界那一刻缓存本来就要重建。
3. **子任务与后台只读任务不复制历史**——换个问题和预算，在同一份 Log 上再编译一次。

## 内核只有两个原语

| 原语 | 负责 |
|---|---|
| Log | 只追加的树：id / parentId / seq，先落盘再进内存，读入时丢弃撕裂的尾行 |
| View seam | 每次调用模型前调一次 `view(log, turn) → CompiledView`；内核只检查视图里 toolCall 成对 |

内核不持有消息数组，也不认识块 / 档位 / 预算 / 申领单——那些全在插件里。

## 状态

M1 内核、M2 申领单与预算、M3 阶段与路由、M4 打分器与回放全部收口。设计决策逐条记在 [`docs/notes/implemented/`](./docs/notes/implemented/)，阶段性快照在 [`docs/status/`](./docs/status/)。

- 已有：one-shot CLI、七个第一方工具（带沙箱与网络策略）、Anthropic 与 OpenAI 兼容适配器（DeepSeek / Qwen / Kimi）、编译器（分块、档位、留桩与 `expand`、阶段冻结、缓存校准、换模型路由、子任务）、规则 / 同模型 / Jev 三种打分器与影子打分、离线回放与真机基准
- 真机：DeepSeek flash 在 20 条 Multi-SWE-bench TS/JS 任务上 15/20（干净 clone + 断网 + 隔离），每条约 4 美分；编译器在 flash 上的入口定档 A/B 已收口（不是净收益），见 `docs/status/2026-09-24-sage-status.md`
- 设计文档：[`docs/design/context-compiler.md`](./docs/design/context-compiler.md)
- 总纲与路线：[issue #1](https://github.com/chasey-myagi/sage/issues/1)（M1 内核 → M2 claim 预算与清单 → M3 阶段与路由 → M4 打分器与回放）

## 快速开始

Node 22.18 以上，pnpm。

```sh
pnpm install
export DEEPSEEK_API_KEY=...   # 或 DASHSCOPE / MOONSHOT / OPENAI / ANTHROPIC 任一
node apps/sage/bin/sage.mjs -p "修掉 test/foo.test.ts 里的失败用例"
```

在当前目录跑完一轮，stdout 是一行 `RunSummary` JSON，stderr 是逐轮进度，会话 Log 写在 `.sage/sessions/`。全部旗标见 [`apps/sage/README.md`](./apps/sage/README.md)。

`pnpm check` 是唯一的本地闸门：lint、类型、分层自检、README 自检、Log 清单自检、测试。

## 仓库结构

依赖只从上往下：

| 包 | 职责 |
|---|---|
| [`apps/sage`](./apps/sage) | CLI 壳：解析参数，组装 provider / 插件 / 工具，跑 session；`bench` 子命令 |
| [`packages/plugins`](./packages/plugins) | 编译器、申领单、打分器、路由、子任务、离线回放 |
| [`packages/tools`](./packages/tools) | 第一方工具：read / write / edit / grep / find / ls / bash |
| [`packages/core`](./packages/core) | Log、View seam、session 循环、运行健康度 |
| [`packages/provider`](./packages/provider) | 本仓自己的 `Context` / `Message` / `Tool` / `Usage` 类型面与各家适配器 |

## 基准

离线回放和真机基准手动触发，不进 CI。任务集、环境准备和三个数的口径见 [`bench/README.md`](./bench/README.md)。

## 协作规矩

见 [`AGENTS.md`](./AGENTS.md)。

## License

MIT
