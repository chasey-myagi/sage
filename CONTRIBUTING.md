# Contributing

## 分支规范

- `main` — 稳定发版分支
- `dev` — 日常开发分支

从 `dev` 创建工作分支：

- `feature/<描述>` — 新功能
- `fix/<描述>` — Bug 修复
- `chore/<描述>` — CI、文档、依赖等杂项

## Commit 规范

[Conventional Commits](https://www.conventionalcommits.org/)：

```
<type>(<scope>): <description>

[optional body]
```

| type | 用途 |
|------|------|
| `feat` | 新功能 |
| `fix` | Bug 修复 |
| `refactor` | 重构（不改行为） |
| `docs` | 文档 |
| `test` | 测试 |
| `chore` | 构建、CI、依赖 |

scope: `ai`, `agent-core`, `tui`, `coding-agent`

示例：
```
feat(ai): add Mistral provider
fix(agent-core): handle empty tool call arguments
refactor(tui): replace hand-written markdown parser with pulldown-cmark
fix(coding-agent): validate agent name against path traversal
chore(ci): add Clippy -Dwarnings gate
```

## PR 流程

1. 从 `dev` 创建功能分支
2. 开发 + `cargo test` + `cargo clippy` 通过
3. PR 到 `dev`
4. Review 通过后 squash merge

## 代码规范

- `cargo fmt` — 统一格式
- `cargo clippy` — lint 必须通过
- 错误处理用 `thiserror` (库) / `anyhow` (应用)
- async runtime 统一用 tokio
