# sage

一个编码 agent 的运行时。它的特色不是「又一个能跑 coding agent 的 harness」，而是**上下文是被编译出来、可审计、跨 provider 的产物**。

## 三样别人做不到的

1. **看得见每个 token 为什么在那里**——每次 LLM call 留一份编译清单，写进 log。
2. **阶段边界零代价换模型**——前缀按阶段冻结，边界那一刻缓存本来就要重建。
3. **子任务与后台只读任务不复制历史**——换个问题和预算，在同一份 Log 上再编译一次。

## 内核只有两个原语

| 原语 | 负责 |
|---|---|
| Log | 只追加的历史：id / parentId / seq、持久化水位（HWM）、血缘 |
| View seam | 每次调用模型前调一次 `view(log, turn) → CompiledView`；内核只检查视图里 toolCall 成对 |

内核不持有消息数组，也不认识块 / 档位 / 预算 / 申领单——那些全在插件里。

## 状态

**设计阶段，M1 未开工。**还没有代码。

- 设计文档：[`docs/design/context-compiler.md`](./docs/design/context-compiler.md)
- 总纲与路线：[issue #1](https://github.com/chasey-myagi/sage/issues/1)（M1 内核 → M2 claim 预算与清单 → M3 阶段与路由 → M4 打分器与回放）

## 基准

离线回放和真机基准手动触发，不进 CI。任务集、环境准备和三个数的口径见 [`bench/README.md`](./bench/README.md)。

## 协作规矩

见 [`AGENTS.md`](./AGENTS.md)。

## License

MIT
