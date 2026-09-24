# 开发手册

给第一次在 sage 里改代码的人和 agent。规则条文在 [`AGENTS.md`](../AGENTS.md)，本文讲它们在这个仓库里具体落在哪、为什么这么定。踩过的坑单独在 [`defensive-patterns.md`](./defensive-patterns.md)。

## 工具链

| 用途 | 工具 | 入口 |
|---|---|---|
| 运行时 | Node.js ≥ 22.18 | 根 `package.json` 的 `engines`；`scripts/gen-log-manifest.mjs` 依赖 22.18 起默认开启的类型擦除 |
| 包管理与工作区 | pnpm workspace | `pnpm-workspace.yaml`；包之间用 `workspace:*` 互引 |
| 类型检查 | TypeScript 7，`tsc -b` 按 project references 逐包检查 | `tsconfig.json`（solution 根）、`tsconfig.base.json`（共享选项） |
| 测试 | vitest | `vitest.config.ts` |
| 格式化与 lint | biome | `biome.json` |
| 仓库自检 | `scripts/*.mjs`，零依赖 Node 脚本 | 见下文「闸门」 |

运行时依赖为零：HTTP 用全局 `fetch`，文件、子进程、路径用 `node:*`。新增运行时依赖要过 [`AGENTS.md`](../AGENTS.md) 「优先用 Node 内置和维护良好的依赖」那一条。

## 源码不编译，直接跑

各包的 `package.json` 把 `exports` 指向 `src/*.ts`，不产出 `dist` 给运行时用。CLI 入口 `apps/sage/bin/sage.mjs` 带 `--experimental-transform-types` 重启自己，让 Node 在加载时擦掉类型；再注册 `apps/sage/src/resolve-ts.mjs`，把相对导入里的 `./x.js` 回落到 `./x.ts`。

由此带来三条硬约束：

- 相对导入写 `.js` 后缀（`"module": "NodeNext"` 的要求，TS 检查时自己映射回 `.ts`）。
- 只作类型用的导入写 `import type`（`isolatedModules` 要求，否则擦类型后留下一个找不到的运行时导入）。
- 不用 `enum` 和 `namespace`：它们会生成运行时代码，也让 JSON 化的 Log 值多一层映射。用字符串字面量联合和 `as const` 对象代替。

`tsc -b` 仍会往各包 `dist/` 写声明与 JS，只有 `scripts/gen-log-manifest.mjs` 读它。

## 包的布局

```
packages/<name>/
  package.json     name、exports（公开出口）、workspace 依赖
  tsconfig.json    extends base；references 列出它依赖的兄弟包
  README.md        必须有 ## Model Experience 三小节（check:readme 检查）
  src/index.ts     桶文件：只 re-export，不写逻辑；没在这里导出的东西，别的包用不到
  src/...          一个文件一个职责
  test/            *.test.ts；回归放 test/regressions/
```

`@sage/provider` 开了两个出口：`.` 给适配器实现，`./contract` 只放类型和纯函数。core 只能 import 后者。要给包加第二个出口，先想清楚它对调用方承诺什么。

依赖方向与禁止项由 `scripts/check-layers.mjs` 的 `RULES` 表执行，改分层先改这张表。

## 常见改动落在哪

| 要做的事 | 改哪里 | 同一个 PR 还要做 |
|---|---|---|
| 加一个工具 | `packages/tools/src/<tool>.ts` 写 `create<Tool>(...)` 返回 `HarnessTool`；在 `packages/tools/src/index.ts` 的 `createTools` 里加入 | 工具失败返回 `fail(...)`，不抛；声明 `accesses` 与三个危险标志；tools README 的 Model Experience |
| 加一种 Log 条目 | `packages/core/src/log/kinds.ts` 里 `registerEntryKind`，带守卫与 `doc` | `pnpm gen:log-manifest` 更新 `docs/log-manifest.md`；若模型可见，按 L4 说明它如何从 Log 重建 |
| 加一个 claim | `packages/plugins/src/claims/<claim>.ts` 返回 `ClaimProvider`；在 `plugins/src/claims/index.ts` 导出 | 在 `apps/sage` 装配处接入；文本进 Log 只经 manifest |
| 加一个 provider 预设 | `packages/provider/src/presets.ts` 与 `catalog.ts` | 定价写 `source` 出处；`apps/sage/README.md` 的 provider 表 |
| 加一个 CLI 旗标 | `apps/sage` 的参数表 | `usage` 文本、`apps/sage/README.md`；`bench run` 是否透传要明确 |
| 修一个有 issue 的 bug | 先写 `test/regressions/issue-<n>.test.ts` 钉红灯 | Agent Note 的 Testing 段写红灯证据 |

## 规范条款的理由

只解释不显然的几条。

**同进程信任类型，边界处验货。**模型给的工具参数、JSONL 行、HTTP 响应体会说谎，TS 类型管不到它们，所以从 `unknown` 开始逐层验证。同一进程里函数 A 把类型化的值交给函数 B，再验一遍只会制造死分支和假测试。

**失败是值。**模型调用失败、工具参数错误、路径越界都是 agent 的日常，要交还给模型或记进 Log，而不是让异常冒到 `session.run` 把整段会话终止（[工具错误不终止会话](./notes/implemented/2026-09-23-tool-error-no-kill.md) 修的就是这个坑）。归一化时丢掉分类字段是同一类问题的另一面：上游知道「这是传输错误」，下游只能靠猜文案。

**状态不进字符串。**`reason` 是给人和 Log 看的说明，拼写会随 PR 变化；拿它 `includes` 做分支判断，改个措辞逻辑就悄悄失效，测试还未必覆盖。需要机器判断的信息加一个结构化字段。

**一个事实一个定义。**`isRecord` 曾在二十多个文件里各写一份，sleep 与退避公式在 core 与 provider 各一份。复制品会各自演化，出了 bug 只修到一份。

**文件头只写契约。**文件头是每个 PR 最顺手的落笔处，结果变成按时间追加的行为清单：`compiler.ts` 的文件头一度 113 行，读者找不到某一段对应哪段代码。行为细节写在实现它的函数上，文件头只留不会随 PR 变的承诺。

**具名常量。**`40000`、`80`、`0.9` 这样的数字同时出现在 CLI、bench、内核里时，改一处漏一处就是行为分叉。具名常量同时回答了「这个数是什么意思」。

## 闸门

`pnpm check` 依次运行，全部通过 PR 才能合：

1. `check:lint`：biome 格式与 lint。
2. `check:types`：`tsc -b`。
3. `check:layers`：分层规则表。
4. `check:readme`：每个包 README 的 Model Experience 三小节；豁免写在 `scripts/readme-exemptions.json`，必须给理由。
5. `check:log-manifest`：`docs/log-manifest.md` 与注册表一致。
6. `test`：vitest。

整套约 20 秒，推送前直接跑全量。迭代时先跑相关的单个测试文件：`pnpm vitest run packages/core/test/session/loop.test.ts`。

## 提交与 PR

- 分支：`<类型>/<主题>`，例如 `fix/transport-flag`、`refactor/cli-args`、`docs/dev-standard`。
- commit 标题：`<类型>(<包>[,<包>]): <中文一句话>`，类型取 `feat` / `fix` / `refactor` / `test` / `docs` / `chore`。正文用中文短句写问题、改法、刻意没做的事。
- 一个 PR 一件事。重构和行为修改分开提：重构 PR 不改任何测试断言，这就是它没改行为的证据。
- 非平凡改动带一份 Agent Note：`docs/notes/implemented/<日期>-<主题>.md`，按 [`TEMPLATE.md`](./notes/TEMPLATE.md) 写。只是提议、还没做的放 `docs/notes/proposed/`。
