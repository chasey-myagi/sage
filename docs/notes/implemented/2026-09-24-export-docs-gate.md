# 导出 JSDoc 自检

## Problem

规范要求每个导出符号有 JSDoc，但只靠评审维持。清理之前，源码约 180 个导出声明没有任何文档注释；函数的行为说明全堆在文件头里。没有闸门，新代码会回到老样子。

## Decision

新增 `scripts/check-export-docs.mjs`，接进 `pnpm check`（在分层自检之后）。扫描 `packages/*/src` 与 `apps/*/src` 下的 `.ts` 文件，找以 `export` 开头的 `function`、`const`、`let`、`class`、`interface`、`type` 声明，要求向上最近的非空行结束一个以 `/**` 开头的块注释。re-export（`export {`、`export *`、`export type {`）不查，它们的文档在声明处。同名函数的连续重载签名只要求第一个有注释。违规时逐条输出 `文件:行 导出缺少 JSDoc：名字` 并退出 1。

只查「有没有」，不查写得好不好：内容质量由 `sage-prose-standard` 与评审负责。

## Alternatives considered

用 TypeScript 编译器 API 精确遍历导出。仓库用的 TypeScript 7 是原生实现，JS 侧编译器 API 不是稳定依赖；而本仓代码统一由 biome 格式化，导出声明总在行首，按行匹配足够准确，且与其余自检脚本一样零依赖。

用 eslint 的 jsdoc 插件。要为一条规则引入 eslint 及其插件链，与 biome 并存两套 lint，成本大于收益。

## Consequences

新增导出必须同时写 JSDoc，否则 `pnpm check` 失败。按行匹配的限制：多行 `export` 修饰或把 `export` 写在装饰器之后的声明不会被识别；本仓目前没有这样的写法，出现时再扩展正则。

## Testing

`scripts/__tests__/check-export-docs.test.ts` 覆盖六种声明、普通块注释与行注释不算、re-export 跳过、重载只要一个、空行分隔、CLI 退出码 0 与 1、只扫 src 不扫 test。清理完成后的仓库上 `pnpm check:export-docs` 输出 0 条违规。
