# sage 状态记录 · 2026-09-24 02:40 CST

> 快照：main `22717ea`（PR #88）。本文只记「到这一刻做到了什么、真机数字是什么、下一步建议」；每张票的动机与取舍在 `docs/notes/implemented/`，设计在 `docs/design/context-compiler.md`。

## 1. 完成情况

| 里程碑 | 状态 | 证据 |
|---|---|---|
| M1–M4 内核与插件（Log、View seam、claims、块/档次、stub+expand、阶段与成本不等式、路由、子任务、打分器、回放与 bench） | 全部合入（2026-09-23） | issue #1 评论；`docs/notes/implemented/2026-09-22-*.md` |
| DeepSeek 适配与真机可用 | 合入 | #45、#47、#54、#63、#64、#85（重试/视界/轮数上限/冻结/校准） |
| 可信 bench 基建 | 合入 | #52 任务集与 `testPatch`；#60 clone 只留 baseCommit；#61 工具断网；#79 工具错误不杀会话；#80/#83 工作区隔离 |
| 打分器可用性 | 合入 | #69/#71 指令与解析；#65 影子打分；#73 入口定档；#77 jev 端点可配；#87 摘要规范 |
| 代码规模 | — | TS 源码+测试约 3.6 万行，`pnpm check` 412 项测试 |

开放票：#62（JSONL 每行 fsync，CI flaky，非阻塞）、#1（总纲）。

## 2. 真机数字（DeepSeek flash，可信版：干净 clone + 断网 + 隔离）

| 实验 | 结果 | 费用 |
|---|---|---|
| 第一批 10 条 passthrough | 9/10 | $0.29 |
| 第二批 10 条 passthrough（7 个新仓库） | 6/10 | $0.55 |
| 第一批 compiler（预算 40000，默认） | 9/10，淘汰 0 / 重建 0 | $0.41 |
| 第一批 compiler + 同模型打分器入口定档（四次） | 稳定 7/10 | $0.33–0.51 |
| DeepSeek V4 Pro passthrough | 7/10 | $1.43 |

三方对照（vue-10289、darkreader-7241，干净 clone）：sage 25s/89s，Cursor grok 82s/139s，Opus 5.5 子 agent 32s/104s，三家都过；github-readme-stats 对联网 agent 无区分度。

## 3. 已确认的结论

1. **在 DeepSeek flash 上编译器不出手是对的**：缓存读只有全价 2%，阶段重建回本 37–106 轮；预算是软的；打分器入口定档稳定少解决 2 条且费用 +75%。编译器的价值要在缓存读贵的 provider 或超长会话上验证。
2. **评测防作弊三道门缺一不可**：clone 只留 baseCommit（agent 会 `git log --all` 找上游修复）、bash 断网（会 curl GitHub API）、工作区隔离（会 `ls` 隔壁临时目录）。仍堵不住的渠道：node_modules 里捆绑的仓库自身新版本。
3. **flash 比 V4 Pro 强且便宜五倍**（用户判断得到证实）。
4. **harness 不揣测完成与否**：无工具调用即结束是刻意设计；追问属于 goal 机制，用户否决。
5. 早停、隐藏快照逐字节要求、自认通过但隐藏测试不符，是 flash 失败的三种主形态。

## 4. 未入库的产物

- 第二批任务集 `ts-mini-2.json`（来源 Multi-SWE-RL，本机 fail→pass 全验）与 Multi-SWE-RL 拉取方式，待进 `bench/tasks/`。
- 逐轮审计脚本 `audit.mjs`、三方对照 harness `compare.mjs`、tee-adapter 排障脚本，在开发机 devkit `.dev-tmp/` 下。

## 5. 下一步建议

1. 入库第二批任务集与拉取脚本；修 #62。
2. `runHealth` 加早停计数（最后一轮无工具调用且工作区有未验证改动），bench 单独标 `stopped-early`。只记录不干预。
3. 编译器价值再验证：本地 LLM2Jev 打分器（需 Linux+NVIDIA；`--scorer jev` 已支持 `JEV_BASE_URL/JEV_API_KEY/JEV_MODEL`）把打分成本压到接近零后重跑入口定档；或在缓存读贵的 provider 上跑同一套 bench。
4. RSI（字节 Seed Aspire/S³Gym/HarnessDev）只研究不实现，继续攒材料。
