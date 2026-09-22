# 仓库骨架

## Problem

空仓没有可安装的 workspace。分层纪律和 README 三问只写在文档里，push 之前没有一个入口能把类型、测试和这两道纪律一起挡住。

## Decision

用 pnpm workspace 搭 `packages/{provider,core,plugins,tools}` 与 `apps/sage`。类型检查用 `tsc -b`，测试用 vitest，lint 和 format 只用 biome。`pnpm check` 串起 lint、类型、分层脚本、README 三问和测试。分层规则只写在 `scripts/check-layers.mjs` 顶部的表里，用正则扫 `import` / `export from` / `import()`。`packages/provider/src/contract` 只放纯类型，不 import 任何外部模块。五个包的 README 都回答 Model Experience 三问，豁免表先留空数组。

## Alternatives considered

eslint 加 prettier 能把 lint 和 format 拆开，但要两套配置、两套依赖，骨架阶段 biome 一个工具就够。用 TypeScript compiler API 或 babel 做 AST 能少误报，但会多一个解析依赖，和「正则即可、不加别的依赖」冲突。让 contract 依赖 core 或 node 类型能少写重复声明，但 contract 的纯度就是零外部 import，类型面才能被单独引用。

## Consequences

以后每加一种跨包 import 都要过这张规则表。正则会把注释和字符串里的 import 文本也算进去，遇到误报就收紧正则，不另做忽略名单。contract 不能 import node 或第三方，包括 `node:fs`。README 缺三问且不在豁免表里时检查失败；豁免条目的 reason 为空同样失败。

## Testing

`pnpm check` 退出码为 0。临时在 `packages/core/src` 里 import `@sage/plugins`、在 `packages/provider/src/contract` 里 import `node:fs`、删掉某个未豁免包的 `## Model Experience`，对应脚本退出码非 0。把该包写入豁免表并带上非空 reason 后，README 检查退出码为 0。
