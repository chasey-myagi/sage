# CC Sub-Agent 架构分析

## 文件清单

### 核心文件
- `tools/AgentTool/AgentTool.tsx` - Agent Tool UI 和执行主控
- `tools/AgentTool/runAgent.ts` - Sub-agent 运行引擎（async generator）
- `tools/AgentTool/forkSubagent.ts` - Fork 机制实现
- `tools/shared/spawnMultiAgent.ts` - 多 agent 并行生成和管理
- `utils/forkedAgent.ts` - Sub-agent 上下文创建和隔离
- `tools/AgentTool/built-in/` - 内置 agent 定义（6 种）
- `tools/AgentTool/loadAgentsDir.ts` - Agent 定义加载和管理

## 1. Agent Tool 参数接口

### runAgent 函数签名
```typescript
export async function* runAgent({
  agentDefinition,          // Agent 定义：类型、工具、系统提示、MCP 服务
  promptMessages,           // 初始提示消息数组
  toolUseContext,          // 工具执行上下文（包含 AppState、options、abort 控制）
  canUseTool,              // 权限检查函数
  isAsync,                 // 异步执行标志
  canShowPermissionPrompts, // 是否显示权限提示（默认 !isAsync）
  forkContextMessages,     // Fork 时继承的父消息
  querySource,             // 查询来源标识
  override,                // 覆盖选项
    - userContext          // 用户上下文覆盖
    - systemContext        // 系统上下文覆盖
    - systemPrompt         // 系统提示词覆盖
    - abortController      // 中止控制器
    - agentId              // Agent ID 覆盖
  model,                   // 模型选择（opus/sonnet/haiku）
  maxTurns,                // 最大轮数限制
  preserveToolUseResults,  // 是否保留 tool use 结果
  availableTools,          // 可用工具列表（预计算）
  allowedTools,            // 允许的工具列表（权限过滤）
  onCacheSafeParams,       // Cache 参数回调
  contentReplacementState,  // 内容替换状态
  useExactTools,           // 是否精确使用工具列表
  worktreePath,            // Worktree 路径（隔离工作目录）
  description,             // 任务描述
  transcriptSubdir,        // 转录子目录
  onQueryProgress,         // 查询进度回调
})
```

### AgentDefinition 接口
```typescript
interface AgentDefinition {
  agentType: string              // Agent 类型标识
  whenToUse: string              // 使用说明
  tools: string[]                // 可用工具列表（支持 ['*'] 继承全部）
  maxTurns?: number              // 最大轮数
  model?: 'inherit' | ModelAlias  // 模型选择
  permissionMode?: PermissionMode // 权限模式
  source: 'built-in' | 'custom' | 'plugin'
  mcpServers?: Array<string | Record<string, MCPServerConfig>>  // MCP 服务定义
  getSystemPrompt: () => SystemPrompt
}
```

## 2. 子 Agent 上下文隔离机制

### createSubagentContext 的隔离策略

#### 2.1 完全隔离的字段（每个子 agent 独占副本）
```typescript
readFileState        // 文件状态缓存（clone）
contentReplacementState  // 内容替换状态（clone）
abortController      // 中止控制器（新建或继承）
agentId              // 唯一 agent ID（新建）
queryTracking        // 查询链追踪（新建 chainId，深度 +1）
messages             // 消息历史（可覆盖或继承）
```

#### 2.2 可选共享的字段（用于交互式子 agent）
```typescript
shareAbortController       // 共享父 abort 控制器（交互式 agent 需要）
shareSetAppState           // 共享状态设置回调
shareSetResponseLength     // 共享响应长度设置
setAppStateForTasks        // 始终共享（后台任务注册必需）
```

#### 2.3 隔离的权限上下文
- `shouldAvoidPermissionPrompts = true`（默认）
- 异步子 agent 不显示权限提示
- 交互式子 agent（shareAbortController）继承父权限模式

#### 2.4 隔离的本地状态
```typescript
nestedMemoryAttachmentTriggers
loadedNestedMemoryPaths
dynamicSkillDirTriggers
discoveredSkillNames
localDenialTracking      // 权限拒绝追踪（独立计数）
```

## 3. Fork 机制（隐式继承）

### Fork 特性（实验性，feature gate）
```typescript
isForkSubagentEnabled(): boolean {
  if (feature('FORK_SUBAGENT')) {
    if (isCoordinatorMode()) return false     // 与协调器模式互斥
    if (getIsNonInteractiveSession()) return false
    return true
  }
  return false
}
```

### Fork Agent 定义
```typescript
const FORK_AGENT = {
  agentType: 'fork',
  tools: ['*'],              // 继承父所有工具
  maxTurns: 200,
  model: 'inherit',          // 继承父模型
  permissionMode: 'bubble',  // 权限提示冒泡到父
  useExactTools: true,       // 精确工具匹配（prompt cache）
}
```

### Fork 的消息上下文处理
1. **保留完整的父消息**：所有 assistant 消息的 tool_use 块、thinking、text
2. **占位符替换**：所有 tool_result 块替换为 `"Fork started — processing in background"`
3. **字节级一致性**：所有 fork child 产生完全相同的 API request prefix
4. **Prompt cache 共享**：同一父消息前缀可被多个 fork child 共享

### 防护机制：递归 fork 检测
```typescript
isInForkChild(messages): boolean {
  return messages.some(m => {
    if (m.type !== 'user') return false
    return m.message.content.some(block =>
      block.type === 'text' && 
      block.text.includes(`<fork_boilerplate_tag>`)
    )
  })
}
```

## 4. 并行执行和结果收集

### 多 Agent 生成（spawnMultiAgent）

#### 4.1 生成配置
```typescript
type SpawnTeammateConfig = {
  name: string              // Agent 名称
  prompt: string            // 任务提示词
  team_name?: string        // 团队名称（生成唯一名称）
  cwd?: string              // 工作目录
  model?: string            // 模型选择（'inherit' 支持）
  agent_type?: string       // Agent 类型
  description?: string      // 任务描述
  use_splitpane?: boolean   // 分窗格显示
  plan_mode_required?: boolean
}
```

#### 4.2 后端支持
- **Tmux 后端**：多窗格/窗口管理
- **InProcess 后端**：内进程运行（共享状态）
- **Pane 后端**：分窗格 UI
- **自动检测**：iTerm2、VS Code 等环境

#### 4.3 唯一名称生成
```typescript
generateUniqueTeammateName(baseName, teamName): string {
  // 检查团队成员列表中的重复名称
  // 如果存在重复，追加数字后缀（tester-2, tester-3, ...）
}
```

#### 4.4 环境变量继承
```typescript
buildInheritedCliFlags({
  planModeRequired,
  permissionMode
}) {
  // 继承权限模式（plan 模式优先）
  // 继承 --model 覆盖
  // 继承 --settings 路径
  // 继承 --plugin-dir（所有内联插件）
  // 继承 --chrome / --no-chrome 标志
}
```

### 结果收集模式

#### 异步执行（后台任务通知）
- 所有 agent spawn 异步运行在后台
- 通过 `<task-notification>` 统一交互模型
- 支持多个并行任务

#### InProcess 模式（直接调用）
```typescript
startInProcessTeammate(config) {
  // 返回 Promise<void>
  // 共享 AppState 和 readFileState
  // 支持 sendMessage 交互
}
```

#### 交互模式（Tmux + InProcess）
- 团队成员可通过 SendMessage 相互通信
- Leader 可监视成员状态（idle 通知）
- 团队文件追踪成员和任务关系

## 5. 与主 Agent Tool-call 系统的集成

### 5.1 Tool 执行流程
```
AgentTool 调用
  ↓
检查 agent 定义（built-in/custom/plugin）
  ↓
初始化 MCP 服务（agent 特定 + 继承父级）
  ↓
runAgent() 创建 sub-agent 上下文
  ↓
query() 执行 API 调用循环
  ↓
收集 tool_use → canUseTool 检查 → 执行 → tool_result
  ↓
汇聚结果消息到父进程
```

### 5.2 Tool 权限检查
```typescript
canUseTool: CanUseToolFn = (tool: string, context) => {
  // allowedTools 白名单过滤
  // 权限模式检查（auto/acceptEdits/default）
  // 插件权限检查
}
```

### 5.3 权限模式继承
```
parent permissionMode
  ↓
子 agent permissionMode (override or inherit)
  ↓
permissionSetup.ts 初始化会话权限
  ↓
canUseTool 运行时检查
```

### 5.4 Prompt Cache 策略
```typescript
onCacheSafeParams?: (params: CacheSafeParams) => void

type CacheSafeParams = {
  systemPrompt: SystemPrompt
  userContext: { [k: string]: string }
  systemContext: { [k: string]: string }
  toolUseContext: ToolUseContext
  forkContextMessages: Message[]  // Cache key 的一部分
}
```

#### Cache 命中条件
- systemPrompt 字节级相同
- tools 定义相同
- model 相同
- messages（前缀）相同
- thinking config 相同

#### useExactTools 标志
```typescript
useExactTools?: boolean
// 当为 true 时：
// - 使用 availableTools 直接，不经过 resolveAgentTools()
// - 继承父 thinkingConfig
// - 继承父 isNonInteractiveSession
// - 用途：fork path 产生字节级相同的 API 请求前缀
```

## 6. MCP 服务集成

### Agent 特定的 MCP 服务
```typescript
// 在 agent 定义中声明
mcpServers?: Array<
  string |  // 引用已有服务 "stdio" 或 "sse"
  Record<string, MCPServerConfig>  // 内联定义
>
```

### 服务初始化和清理
```typescript
initializeAgentMcpServers(agentDefinition, parentClients) {
  // 新建的客户端（内联定义）在 agent 完成后清理
  // 引用的客户端（字符串名称）保持活跃（父级共享）
  // 返回合并后的服务列表
}
```

## 7. 转录和状态追踪

### 转录管理
```typescript
transcriptSubdir?: string
// 可选子目录分组关联 agents（如 workflows/<runId>）

recordSidechainTranscript(agentId, messages)
// 记录 sidechain 转录用于 resume
```

### 元数据持久化
```typescript
writeAgentMetadata({
  agentId,
  agentType,
  worktreePath,  // 用于 resume 恢复工作目录
  description,   // 原始任务描述
  model,
  timestamp,
})
```

### 查询追踪
```typescript
queryTracking: {
  chainId: UUID,           // 唯一查询链 ID
  depth: number,           // 嵌套深度（-1=root, 0=direct, 1+=nested）
}
```

## 8. 生命周期管理

### 启动阶段
1. 验证 agent 定义（校验工具列表、权限模式）
2. 初始化 MCP 服务
3. 创建隔离上下文
4. 调用 subagent 启动 hooks（frontmatter 定义）
5. 检查权限和模型访问

### 运行阶段
1. Query 循环：消息输入 → API 调用 → 工具执行 → 结果收集
2. 权限检查：每个 tool_use 检查 allowedTools 和权限模式
3. Abort 处理：内部或外部中止都触发清理
4. 进度回调：onQueryProgress() 检测长时间无进展

### 清理阶段
1. 记录转录（除非 skipTranscript）
2. 清理 agent 特定的 MCP 服务
3. 杀死后台 shell 任务
4. 清除 session hooks
5. 返回最终消息和使用统计

## 9. 数据流示例

### Fork 场景
```
Parent Agent 执行
  ↓ AgentTool(prompt="分析代码...")
  ↓ [省略 subagent_type] → 触发 fork
  ↓
创建 Fork Child
  ├─ 继承父消息 + fork boilerplate
  ├─ 占位符替换所有 tool_result
  ├─ 精确工具继承（useExactTools=true）
  └─ 后台异步执行
  ↓ query() loop
  ↓ 收集 fork 结果
  ↓ <task-notification> 返回给用户
```

### Team/Spawn 场景
```
Leader Agent 执行
  ↓ TeammateTool(name="researcher", prompt="研究...")
  ↓
生成 Teammate
  ├─ 生成唯一名称
  ├─ 选择后端（Tmux/InProcess）
  ├─ 建立 Tmux 窗格或启动 InProcess runner
  ├─ 初始化团队文件
  └─ 开始新进程（或线程）
  ↓
Teammate 执行上下文
  ├─ 继承 leader 权限模式
  ├─ 新 agent ID 和 query chain
  ├─ 隔离的 AppState（InProcess）
  └─ 后台任务共享（setAppStateForTasks）
  ↓
结果交付
  ├─ Sidechain 转录记录
  ├─ Idle 通知（teammate 等待指令）
  └─ SendMessage 交互
```

## 10. 关键设计决策

### 1. Prompt Cache 一致性
**决策**：完全克隆 contentReplacementState 和 forkContextMessages
**理由**：避免 tool_use_id 不匹配导致的替换决策差异
**影响**：Fork child 能精确命中父 cache，减少 API 成本

### 2. 权限隔离默认
**决策**：shouldAvoidPermissionPrompts=true（除非 shareAbortController）
**理由**：异步子 agent 无法显示 UI，显示提示会导致无法应答
**影响**：需要显式的权限白名单或 allowedTools

### 3. Model 继承策略
**决策**：model: 'inherit' 在运行时替换为父模型
**理由**：一致的上下文长度，避免模型能力不对称
**影响**：子 agent 自动采用父模型，无需手动配置

### 4. 团队隔离 vs 共享
**决策**：InProcess teammate 共享 setAppStateForTasks，隔离 setAppState
**理由**：后台任务必须注册到根存储，否则成为僵尸进程
**影响**：InProcess 之间可共享后台任务但保持状态隔离

### 5. MCP 服务生命周期
**决策**：内联定义的服务在 agent 完成后清理，引用的保持活跃
**理由**：避免频繁创建销毁，支持跨 agent 共享
**影响**：前置定义 MCP 配置可被多个 agent 复用

## 11. 已知限制

1. **Fork 深度**：防护检测禁止递归 fork，fork child 无法再 fork
2. **权限冒泡**：permissionMode:'bubble' 仅在交互式 agent 有意义
3. **状态共享陷阱**：setAppState 无-op 可能导致权限拒绝堆积
4. **Worktree 隔离**：worktreePath 需要手动传递（不自动推导）
5. **Transcript 性能**：记录大量消息可能导致 I/O 开销

## 12. Rust 移植建议

### 核心组件映射
```
CC TypeScript          → Sage Rust
─────────────────────────────────
AgentDefinition       → struct AgentDef { ... }
ToolUseContext        → struct ToolContext { ... }
createSubagentContext → fn create_subagent_context() { ... }
runAgent() generator  → async fn run_agent() -> BoxStream<Message> { ... }
forkedAgent.ts        → module forked { pub struct CacheSafeParams { ... } }
spawnMultiAgent       → mod spawn { pub fn spawn_agent() { ... } }
```

### 关键实现优先级
1. **Priority 1**：
   - AgentDefinition 和参数解析
   - ToolUseContext 的隔离克隆
   - Prompt cache 参数构建（CacheSafeParams）

2. **Priority 2**：
   - Fork 机制（消息占位符替换）
   - MCP 服务初始化清理
   - 权限检查和 allowedTools 过滤

3. **Priority 3**：
   - 多 agent 并行管理（tmux 集成可选）
   - 转录和元数据持久化
   - 团队文件和消息交互

### 性能考虑
- 消息克隆：只克隆必要的上下文（使用 Cow<> 等）
- 缓存：AgentDefinition 应缓存（MCP 初始化昂贵）
- 异步：使用 tokio 和 async/await，避免阻塞
- 流式：runAgent 应返回 stream 而不是 Vec，支持流式处理

---
*分析完成时间：2026-04-21*
*涵盖源码范围：src/tools/AgentTool, src/tools/shared, src/utils*
