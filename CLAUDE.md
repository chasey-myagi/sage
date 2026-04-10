# agent-caster

Agent OS 的 Executor Caster，基于 pi-mono + Rune Runtime。

## 项目结构

```
src/
├── index.ts            # Caster 注册 + 启动
├── runner.ts           # pi-mono Agent 构建 + 执行
├── tools.ts            # 工具创建（基于 pi-mono 内置工具）
└── types.ts            # AgentConfig 类型定义
```

## Build & Run

```bash
pnpm install
pnpm build
pnpm start --runtime localhost:50070
pnpm dev --runtime localhost:50070
```

## 依赖

- pi-mono (`@mariozechner/pi-agent-core`, `@mariozechner/pi-ai`, `@mariozechner/pi-coding-agent`)
- Rune TS SDK (`@rune-framework/caster`)
- Rune Runtime 需先启动

## Git 工作流

- `dev` — 日常开发
- `main` — 发版
- Conventional Commits: `feat(scope): description`
- scope: `caster`, `runner`, `tools`, `config`
