# CC MCP 架构分析报告

## 执行摘要

本报告分析了 Claude Code 的 MCP (Model Context Protocol) 实现架构，为 Rune 生态的 Rust 移植提供技术方案。关键发现：

1. **MCP 通过统一 Tool 接口集成** - 内置工具和 MCP 工具共享同一接口
2. **三层架构** - 协议层（SDK）、连接层（Client）、应用层（Tool 转换）
3. **安全重心** - 权限检查、域名验证、超时控制、内容大小限制

---

## 1. MCP Stdio 协议实现

### 1.1 协议栈

**位置**: `/Users/chasey/Dev/cc/external/claude-code/src/services/mcp/`

**关键文件**:
- `client.ts` (3348 行) - MCP 客户端核心实现
- `types.ts` - 类型定义和配置 schema
- `InProcessTransport.ts` - 进程内传输
- `SdkControlTransport.ts` - SDK 控制传输

### 1.2 消息流程

```
User Input
    ↓
Tool Registry (client.ts:1743)
    ↓ [tools/list]
MCP Server (Stdio Transport)
    ↓
List Tools Response → Tool 对象数组
    ↓
buildMcpToolName() 前缀处理
    ↓
转换为 Tool 接口 (client.ts:1766-1897)
    ↓
Permission Check → 权限控制
    ↓
Tool Call (client.ts:3029)
    ↓ [tools/call]
MCP Server
    ↓
Call Tool Response
```

### 1.3 协议消息格式

#### Initialize（自动处理）
由 `@modelcontextprotocol/sdk` 的 `Client` 类处理。

#### List Tools 请求/响应
**方法**: `tools/list`

**响应格式** (client.ts:1752-1755)：
```typescript
{
  tools: Array<{
    name: string,           // 工具名称
    description?: string,   // 工具描述
    inputSchema: {          // JSON Schema
      type: 'object',
      properties: {...},
      required: [...]
    },
    annotations?: {
      "anthropic/searchHint"?: string  // 搜索提示
    },
    _meta?: Record<string, unknown>  // 扩展元数据
  }>
}
```

**关键处理**:
- `recursivelySanitizeUnicode()` - 清理 Unicode 问题
- `MAX_MCP_DESCRIPTION_LENGTH = 2048` - 描述截断
- `TOOL_NAME_PREFIX = 'mcp__'` - 工具名称前缀

#### Call Tool 请求/响应
**方法**: `tools/call`

**请求格式** (client.ts:3093-3096)：
```typescript
{
  name: string,           // 完整工具名称
  arguments: Record<...>, // 工具参数
  _meta?: {...}          // 扩展元数据
}
```

**响应格式**：
```typescript
{
  content: Array<{
    type: 'text' | 'image' | 'document',
    text?: string,
    source?: {type: 'base64' | 'url', ...}
  }>,
  isError?: boolean
}
```

**进度报告** (client.ts:3102-3114)：
- `onProgress` 回调
- 进度消息格式: `{progress: number, total?: number, message?: string}`

### 1.4 Stdio 配置架构

**配置 Schema** (types.ts:28-35)：
```typescript
McpStdioServerConfigSchema = z.object({
  type: z.literal('stdio').optional(),
  command: z.string().min(1),      // 可执行程序路径
  args: z.array(z.string()).default([]),      // 命令行参数
  env: z.record(z.string(), z.string()).optional(),  // 环境变量
})
```

**连接函数** (client.ts:595)：
```typescript
connectToServer(config: McpServerConfig): Promise<MCPServerConnection>
```

支持的传输类型: `'stdio' | 'sse' | 'sse-ide' | 'http' | 'ws' | 'sdk'`

### 1.5 超时与重试

**超时控制** (client.ts:3070)：
```typescript
getMcpToolTimeoutMs() // 默认值：~27.8 小时（100,000 秒）
```

可配置的超时机制用于防止工具执行无限期挂起。

---

## 2. 工具注册与统一机制

### 2.1 Tool 接口定义

**位置**: `/Users/chasey/Dev/cc/external/claude-code/src/Tool.ts` (行 362-630)

**核心属性**:

| 属性 | 类型 | 必需 | 说明 |
|-----|------|------|------|
| `name` | string | ✓ | 工具唯一标识 |
| `call` | async function | ✓ | `(input, context) => Promise<ToolResult>` |
| `inputSchema` | Zod \| JSON Schema | ✓ | 输入验证 |
| `outputSchema` | Zod Type | ✓ | 输出验证 |
| `description` | async function | ✓ | 动态生成描述 |
| `prompt` | async function | ✓ | 系统提示内容 |
| `checkPermissions` | async function | ✓ | 权限检查逻辑 |
| `maxResultSizeChars` | number | ✓ | 结果大小限制 |
| `isMcp` | boolean | | 标记 MCP 工具 |
| `mcpInfo` | {serverName, toolName} | | MCP 源信息 |
| `shouldDefer` | boolean | | 延迟执行标记 |
| `isConcurrencySafe` | boolean | | 并发安全性 |
| `isReadOnly` | boolean | | 只读标记 |
| `isDestructive` | boolean | | 破坏性操作标记 |

### 2.2 buildTool 工厂函数

**位置**: Tool.ts 行 783-791

```typescript
export function buildTool<D extends AnyToolDef>(def: D): BuiltTool<D> {
  return {
    ...TOOL_DEFAULTS,           // 应用默认值
    userFacingName: () => def.name,
    ...def,                      // 用户定义覆盖
  } as BuiltTool<D>
}
```

**默认值应用** (行 757-769)：
- `isEnabled()` → `true`
- `isConcurrencySafe()` → `false`
- `isReadOnly()` → `false`
- `isDestructive()` → `false`
- `checkPermissions()` → `{ behavior: 'allow' }`

### 2.3 MCP 工具转换流程

**位置**: client.ts 行 1766-1897

```typescript
// 核心转换逻辑
const toolsToProcess: Tool[] = toolsFromMCP.map((mcpTool) => ({
  ...MCPTool,  // 基类
  name: buildMcpToolName(client.name, mcpTool.name),  // 前缀处理
  mcpInfo: { serverName: client.name, toolName: mcpTool.name },
  isMcp: true,
  description: () => mcpTool.description ?? '',
  inputJSONSchema: mcpTool.inputSchema,
  
  // 工具调用委托
  async call(input, context) {
    const connectedClient = await ensureConnectedClient(...)
    return await callMCPToolWithUrlElicitationRetry(...)
  },
  
  // 权限检查
  async checkPermissions(input, context) {
    // 权限逻辑...
  }
}))
```

**名称前缀处理**:
```typescript
buildMcpToolName(serverName: string, toolName: string): string
// 结果: "mcp__<serverName>__<toolName>"
// 例: "mcp__brave_search__search_query"

// 可选: 无前缀模式 (CLAUDE_AGENT_SDK_MCP_NO_PREFIX)
```

### 2.4 MCPTool 基类

**位置**: `/Users/chasey/Dev/cc/external/claude-code/src/tools/MCPTool/MCPTool.ts`

```typescript
export const MCPTool = buildTool({
  isMcp: true,
  name: 'mcp',  // 被转换覆盖
  maxResultSizeChars: 100_000,
  async description() { return DESCRIPTION; },
  async prompt() { return PROMPT; },
  get inputSchema() { return z.object({}).passthrough(); },
  get outputSchema() { return z.string(); },
  async call() { return { data: '' }; },  // 被覆盖
  async checkPermissions(): Promise<PermissionResult> {
    return { 
      behavior: 'passthrough', 
      message: 'MCPTool requires permission.' 
    };
  },
})
```

### 2.5 权限检查统一机制

**权限结果类型**:
```typescript
type PermissionResult = {
  behavior: 'allow' | 'deny' | 'ask' | 'passthrough',
  message?: string,
  updatedInput?: Input,
  suggestions?: Array<{
    type: 'addRules',
    rules: Array<{toolName: string}>,
    behavior: 'allow' | 'deny',
    destination: 'localSettings' | 'projectSettings'
  }>
}
```

所有工具（内置和 MCP）都通过相同接口进行权限检查。

### 2.6 工具搜索支持

**searchHint 字段**:
- 内置工具：Tool 接口中的 `searchHint?: string`
- MCP 工具：从 `tool._meta['anthropic/searchHint']` 提取

---

## 3. WebFetch 工具实现

### 3.1 架构概览

**位置**: `/Users/chasey/Dev/cc/external/claude-code/src/tools/WebFetchTool/`

**文件结构**:
- `WebFetchTool.ts` - 工具定义和核心逻辑
- `utils.ts` - HTTP 获取、HTML 解析、缓存管理
- `preapproved.ts` - 预批准域名列表

### 3.2 缓存系统

**位置**: utils.ts 行 50-83

```typescript
// 主缓存：URL 内容
const URL_CACHE = new LRUCache<string, CacheEntry>({
  maxSize: 50 * 1024 * 1024,  // 50MB
  ttl: 15 * 60 * 1000,        // 15 分钟
})

// 辅助缓存：域名检查
const DOMAIN_CHECK_CACHE = new LRUCache<string, true>({
  max: 128,          // 最多 128 条目
  ttl: 5 * 60 * 1000 // 5 分钟
})
```

**缓存项结构**:
```typescript
interface CacheEntry {
  bytes: number,           // 字节大小
  code: number,            // HTTP 状态码
  codeText: string,        // 状态文本
  content: string,         // Markdown 内容
  contentType: string,     // MIME 类型
  persistedPath?: string,  // 二进制文件路径
  persistedSize?: number,  // 二进制文件大小
}
```

### 3.3 安全检查流程

**位置**: utils.ts 行 176-203

```typescript
async function checkDomainBlocklist(domain: string): Promise<DomainCheckResult> {
  // 调用 Anthropic API
  const response = await axios.get(
    `https://api.anthropic.com/api/web/domain_info?domain=${domain}`,
    { timeout: DOMAIN_CHECK_TIMEOUT_MS }  // 10 秒
  )
  
  if (response.data.can_fetch === true) {
    return { status: 'allowed' }
  } else {
    return { status: 'blocked', reason: response.data.reason }
  }
}
```

**域名缓存**: 5 分钟内重复检查同一域名会使用缓存。

### 3.4 HTTP 获取与重定向处理

**位置**: utils.ts 行 262-329

**关键参数**:
```typescript
const config: AxiosRequestConfig = {
  timeout: 60_000,              // 60 秒超时
  maxRedirects: 0,              // 手动处理重定向
  responseType: 'arraybuffer',  // 二进制响应
  maxContentLength: 10 * 1024 * 1024,  // 10MB 限制
}
```

**重定向处理**:
```typescript
async function getWithPermittedRedirects(
  url: string,
  signal: AbortSignal,
  redirectChecker: (original, redirected) => boolean,
  depth = 0  // 防止无限循环，最多 MAX_REDIRECTS (10)
): Promise<AxiosResponse | RedirectInfo> {
  // 1. 判断重定向是否允许
  // 2. 跟踪重定向链
  // 3. 返回最终响应或重定向信息
}
```

### 3.5 HTML 解析与内容提取

**位置**: utils.ts 行 347-482

**流程**:

1. **URL 验证** (行 139-169)
   - 长度检查: `MAX_URL_LENGTH = 2000`
   - 协议检查: 自动升级 `http://` → `https://`
   - 认证检查: 拒绝含用户名/密码的 URL

2. **HTTP 获取** (行 416-445)
   - axios 请求，支持文本和二进制内容
   - 二进制内容持久化到磁盘

3. **HTML 到 Markdown 转换** (行 456-466)
   ```typescript
   if (contentType.includes('text/html')) {
     const turndownService = await getTurndownService()
     markdownContent = turndownService.turndown(htmlContent)
   } else {
     markdownContent = htmlContent
   }
   ```
   - HTML 解析库: **Turndown** (通过 @mixmark-io/domino)
   - 延迟加载: 节省内存，仅在需要时导入
   - 单例重用: `turndownServicePromise`

4. **内容截断与缓存** (行 468-481)
   ```typescript
   if (markdownContent.length > MAX_MARKDOWN_LENGTH) {
     // 调用 Haiku 模型进行内容摘要
   }
   URL_CACHE.set(url, cacheEntry, { size: contentBytes })
   ```

### 3.6 Haiku 模型后处理

**位置**: utils.ts 行 484-530

```typescript
async function applyPromptToMarkdown(
  userPrompt: string,
  markdownContent: string,
  signal: AbortSignal,
  isNonInteractiveSession: boolean,
  isPreapprovedDomain: boolean
): Promise<string> {
  // 系统提示组合
  // Haiku 模型处理大内容
  // 返回提取的相关内容
}
```

**触发条件**:
- 内容长度 > `MAX_MARKDOWN_LENGTH (100,000)`
- 非预批准域名时使用 Haiku 缩小内容

### 3.7 Tool 定义

**位置**: WebFetchTool.ts 行 66-249

```typescript
export const WebFetchTool = buildTool({
  name: 'web_fetch',
  searchHint: 'fetch and extract content from a URL',
  maxResultSizeChars: 100_000,
  shouldDefer: true,  // 延迟工具
  
  async call({ url, prompt }, { abortController, options }) {
    // 1. 域名检查
    // 2. 缓存查询
    // 3. HTTP 获取
    // 4. HTML 解析
    // 5. 内容处理（可能调用 Haiku）
    return {
      bytes: contentLength,
      code: statusCode,
      codeText: statusText,
      result: markdownContent,
      durationMs: elapsedTime,
      url: finalUrl
    }
  },
  
  async checkPermissions(input, context) {
    // 权限检查逻辑
    // 包括预批准域名优化
  }
})
```

**输入 Schema**:
```typescript
z.strictObject({
  url: z.string().url(),
  prompt: z.string()
})
```

**输出 Schema**:
```typescript
z.object({
  bytes: z.number(),
  code: z.number(),
  codeText: z.string(),
  result: z.string(),
  durationMs: z.number(),
  url: z.string()
})
```

### 3.8 关键配置常量

| 常量 | 值 | 用途 | 行号 |
|-----|-----|------|------|
| MAX_URL_LENGTH | 2,000 | URL 长度限制 | 106 |
| MAX_HTTP_CONTENT_LENGTH | 10 MB | HTTP 响应大小限制 | 112 |
| FETCH_TIMEOUT_MS | 60,000 | 主 HTTP 请求超时 | 116 |
| DOMAIN_CHECK_TIMEOUT_MS | 10,000 | 域名检查超时 | 119 |
| MAX_REDIRECTS | 10 | 重定向最大跳转数 | 125 |
| MAX_MARKDOWN_LENGTH | 100,000 | Markdown 截断阈值 | 128 |
| CACHE_TTL_MS | 900,000 | 缓存 TTL（15 分钟） | 63 |
| MAX_CACHE_SIZE_BYTES | 50 MB | 缓存最大大小 | 64 |

---

## 4. WebSearch 工具实现

### 4.1 架构概览

**位置**: `/Users/chasey/Dev/cc/external/claude-code/src/tools/WebSearchTool/WebSearchTool.ts`

**核心特点**:
- 基于 Claude API 内置 `web_search_20250305` 工具
- 流式处理搜索结果
- 支持域名过滤

### 4.2 输入/输出 Schema

**输入** (行 25-37)：
```typescript
z.strictObject({
  query: z.string().min(2),
  allowed_domains: z.array(z.string()).optional(),
  blocked_domains: z.array(z.string()).optional()
})
```

**输出** (行 56-66)：
```typescript
z.object({
  query: z.string(),
  results: z.array(z.union([
    z.object({
      title: z.string(),
      url: z.string()
    }),
    z.string()  // 文本摘要/答案
  ])),
  durationSeconds: z.number()
})
```

### 4.3 工具 Schema 构建

**位置**: 行 76-84

```typescript
function makeToolSchema(input: Input): BetaWebSearchTool20250305 {
  return {
    type: 'web_search_20250305',
    name: 'web_search',
    allowed_domains: input.allowed_domains,
    blocked_domains: input.blocked_domains,
    max_uses: 8  // 单次调用最多搜索 8 次
  }
}
```

### 4.4 模型调用流程

**位置**: 行 254-400

```typescript
// 1. 构建用户消息
const userMessage: BetaMessageParam = {
  role: 'user',
  content: generateSearchPrompt(input.query)
}

// 2. 调用模型
const queryStream = queryModelWithStreaming({
  messages: [userMessage],
  systemPrompt: [...],
  tools: [],  // 工具由 extraToolSchemas 提供
  extraToolSchemas: [toolSchema],  // web_search_20250305
  signal: abortController.signal,
  options: {
    model: useHaiku ? 'claude-haiku-4.5' : mainModel,
    toolChoice: { type: 'tool', name: 'web_search' }
  }
})

// 3. 流事件处理
for await (const event of queryStream) {
  switch (event.type) {
    case 'content_block_start':
      if (event.contentBlock.type === 'web_search_tool_result') {
        // 处理搜索结果
      }
      break
    
    case 'content_block_delta':
      // 累积输入 JSON
      break
  }
}
```

### 4.5 结果处理

**位置**: 行 86-150

```typescript
function makeOutputFromSearchResponse(
  contentBlocks: BetaContentBlock[],
  query: string,
  durationSeconds: number
): Output {
  const output: Output = {
    query,
    results: [],
    durationSeconds
  }
  
  // 遍历内容块
  for (const block of contentBlocks) {
    if (block.type === 'web_search_tool_result') {
      // 提取: title + url 或错误信息
      const result = block.result as BetaWebSearchToolResult
      output.results.push({
        title: result.title || '',
        url: result.url || ''
      })
    } else if (block.type === 'text') {
      // 添加文本摘要
      output.results.push(block.text)
    }
  }
  
  return output
}
```

### 4.6 权限检查与模型支持检测

**权限检查** (行 209-222)：
```typescript
async checkPermissions(_input): Promise<PermissionResult> {
  return {
    behavior: 'passthrough',
    message: 'WebSearchTool requires permission.',
    suggestions: [{
      type: 'addRules',
      rules: [{ toolName: 'web_search' }],
      behavior: 'allow',
      destination: 'localSettings'
    }]
  }
}
```

**模型支持检测** (行 168-193)：
```typescript
isEnabled() {
  const provider = getAPIProvider()
  const model = getMainLoopModel()
  
  // 仅支持特定提供商和模型
  if (provider === 'firstParty') return true
  if (provider === 'vertex') {
    return model.includes('claude-opus-4') ||
           model.includes('claude-sonnet-4') ||
           model.includes('claude-haiku-4')
  }
  if (provider === 'foundry') return true
  return false
}
```

### 4.7 Tool 定义

**位置**: 行 152-435

```typescript
export const WebSearchTool = buildTool({
  name: 'web_search',
  searchHint: 'search the web for current information',
  maxResultSizeChars: 100_000,
  shouldDefer: true,
  
  async call(input, context) {
    const startTime = Date.now()
    const results = []
    
    // 模型调用与流处理
    for await (const event of queryStream) {
      // ... 事件处理
    }
    
    const durationSeconds = (Date.now() - startTime) / 1000
    return makeOutputFromSearchResponse(contentBlocks, input.query, durationSeconds)
  },
  
  checkPermissions: async (input) => ({ behavior: 'passthrough', ... }),
  isEnabled: () => { /* 模型检查 */ },
  isConcurrencySafe: () => true,
  isReadOnly: () => true
})
```

---

## 5. 关键数据结构与类型

### 5.1 MCPServerConnection 联合类型

**位置**: types.ts 行 180-226

```typescript
type MCPServerConnection =
  | {
      client: MCP.Client,
      name: string,
      type: 'connected',
      capabilities: MCP.ServerCapabilities,
      serverInfo?: MCP.Implementation,
      instructions?: string,
      config: McpServerConfig,
      cleanup: () => Promise<void>
    }
  | { name: string, type: 'failed', config: McpServerConfig, error?: string }
  | { name: string, type: 'needs-auth', config: McpServerConfig }
  | { name: string, type: 'pending', config: McpServerConfig, ... }
  | { name: string, type: 'disabled', config: McpServerConfig }
```

### 5.2 Tool 接口核心属性

**位置**: Tool.ts 行 362-630

完整属性列表见第 2.1 章。

### 5.3 PermissionResult 类型

```typescript
type PermissionResult = {
  behavior: 'allow' | 'deny' | 'ask' | 'passthrough',
  message?: string,
  updatedInput?: Input,
  suggestions?: Array<{
    type: 'addRules' | 'removeRules' | 'modifyRules',
    rules: Array<{ toolName: string, behavior?: string }>,
    behavior: 'allow' | 'deny',
    destination: 'localSettings' | 'projectSettings'
  }>
}
```

---

## 6. 架构设计原则

### 6.1 统一 Tool 接口

所有工具（内置和 MCP）都实现同一 `Tool` 接口，通过以下机制实现：
1. **标记字段**: `isMcp: boolean`
2. **源信息**: `mcpInfo: {serverName, toolName}`
3. **统一调用**: 相同的 `call()` 签名
4. **统一权限**: 相同的 `checkPermissions()` 机制

### 6.2 分层架构

```
应用层 (Tool 接口)
    ↑
工具注册层 (Tool 转换, 名称前缀)
    ↑
连接层 (MCP Client, 缓存, 权限)
    ↑
协议层 (Stdio Transport, SDK)
    ↑
MCP Server (外部进程)
```

### 6.3 安全设计

**多层防御**:
1. **传输层**: Stdio 标准输入/输出，进程隔离
2. **连接层**: 超时控制、内容大小限制
3. **应用层**: 权限检查、域名验证（WebFetch）、模型支持检测（WebSearch）

### 6.4 性能优化

**缓存策略**:
- MCP 工具列表缓存 (LRU)
- WebFetch 内容缓存 (LRU, 50MB, 15 分钟)
- 域名检查缓存 (LRU, 128 条目, 5 分钟)

**延迟加载**:
- Turndown HTML 解析库: 仅在需要时导入
- MCP 工具: 连接时动态列表化

---

## 7. Rust 移植关键点

### 7.1 必须实现

1. **MCP Stdio 传输**
   - 进程启动和通信
   - JSONRPC 2.0 消息处理
   - 错误处理和超时

2. **Tool 接口**
   - 统一接口定义（使用 trait）
   - 权限检查机制
   - 结果验证

3. **工具注册系统**
   - MCP 工具发现
   - 名称前缀处理
   - 内置工具与 MCP 工具的统一

4. **安全机制**
   - 权限检查流程
   - 超时控制
   - 内容大小限制

### 7.2 可选但推荐

1. **缓存层**
   - LRU 缓存（工具列表、内容）
   - TTL 机制
   - 大小限制

2. **WebFetch 工具**
   - HTTP 获取、HTML 解析
   - 重定向处理
   - 域名检查集成

3. **WebSearch 工具**
   - Claude API 集成
   - 流式处理

---

## 8. 关键代码行号索引

| 功能 | 文件 | 行号 |
|-----|-----|------|
| MCP 客户端入口 | client.ts | 595 |
| 工具列表获取 | client.ts | 1743 |
| 工具转换逻辑 | client.ts | 1766-1897 |
| 工具调用 | client.ts | 3029 |
| Tool 接口 | Tool.ts | 362-630 |
| buildTool 工厂 | Tool.ts | 783-791 |
| WebFetch 缓存 | WebFetchTool/utils.ts | 50-83 |
| HTML 解析 | WebFetchTool/utils.ts | 456-466 |
| WebSearch 定义 | WebSearchTool.ts | 152-435 |
| MCP 类型 | types.ts | 1-259 |
