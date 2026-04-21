# Sage v0.2.0 MCP 架构 Rust 移植实现方案

## 执行摘要

本方案提供从 Claude Code TypeScript 实现到 Rune/Rust 的移植路线图。核心策略是**分阶段实现**，从最小可行产品（MVP）开始，逐步完善功能。

**预期工作量**: 
- Phase 1 (MVP): ~2-3 周
- Phase 2 (完整功能): ~2-3 周
- Phase 3 (优化): ~1-2 周

---

## 第一部分：架构决策

### 1.1 Rust 技术栈选择

#### 1.1.1 MCP Stdio 传输

**选项分析**:

| 技术 | 优势 | 劣势 | 推荐 |
|-----|-----|------|------|
| tokio + serde_json | 异步成熟、JSON 生态完善 | 需要手写消息解析 | ✓ 推荐 |
| mcp crate (if exists) | 官方支持（如果存在） | 可能 API 不稳定 | 优先评估 |
| jsonrpc crate | 标准 JSONRPC 实现 | 可能过度设计 | 备选 |

**推荐方案**: 使用 `tokio` 异步运行时 + `serde_json` + `serde` 手写 JSONRPC 消息处理。
- 理由：Rune 生态已有 tokio 依赖，最小化外部依赖。

#### 1.1.2 数据结构与序列化

**选项分析**:

| 方案 | 优势 | 劣势 | 推荐 |
|-----|------|------|------|
| `serde` + `serde_json` | 广泛支持、高性能 | 需要 derive macros | ✓ 推荐 |
| `serde` + `serde_yaml` | 易读配置 | 性能差 | 配置文件用 |
| 手写反序列化 | 完全控制 | 维护负担大 | 不推荐 |

**推荐方案**: `serde` + `serde_json` 为主，配置使用 TOML。

#### 1.1.3 Tool 接口与权限系统

**选项分析**:

| 方案 | 优势 | 劣势 | 推荐 |
|-----|------|------|------|
| trait 对象 (`Box<dyn Tool>`) | 运行时多态，灵活 | 性能开销 | ✓ MVP |
| 枚举 + match | 性能最优 | 扩展性差 | ✓ Phase 2 优化 |
| 宏生成 | 编译期优化 | 复杂度高 | Phase 2+ |

**推荐方案**: Phase 1 使用 trait 对象实现 MVP，Phase 2 评估性能后决定是否转换为枚举或宏。

#### 1.1.4 缓存实现

**选项分析**:

| 库 | 优势 | 劣势 | 推荐 |
|----|-----|------|------|
| `lru` crate | 成熟、简洁 | 无 TTL | ✓ MVP |
| `moka` | 异步支持、TTL | 重量级 | Phase 2 |
| 手写 | 完全控制 | 维护负担 | 不推荐 |

**推荐方案**: Phase 1 使用 `lru` crate，Phase 2 迁移到 `moka` 支持 TTL 和并发。

### 1.2 项目结构

```
sage-rune/
├── Cargo.toml
├── src/
│   ├── lib.rs                    # 库入口
│   ├── mcp/
│   │   ├── mod.rs               # MCP 模块入口
│   │   ├── client.rs            # MCP 客户端（核心）
│   │   ├── transport.rs         # Stdio 传输
│   │   ├── types.rs             # MCP 消息类型定义
│   │   ├── error.rs             # 错误类型
│   │   └── config.rs            # 服务器配置
│   ├── tools/
│   │   ├── mod.rs               # 工具模块入口
│   │   ├── interface.rs         # Tool trait 定义
│   │   ├── registry.rs          # 工具注册表
│   │   ├── permission.rs        # 权限系统
│   │   ├── builtin/
│   │   │   ├── mod.rs           # 内置工具模块
│   │   │   ├── web_fetch.rs     # WebFetch 工具
│   │   │   └── web_search.rs    # WebSearch 工具
│   │   └── mcp_tool.rs          # MCP 工具适配层
│   ├── cache/
│   │   ├── mod.rs               # 缓存模块入口
│   │   ├── lru.rs               # LRU 缓存实现
│   │   └── ttl.rs               # TTL 缓存（Phase 2）
│   └── utils/
│       ├── mod.rs               # 工具函数
│       ├── http.rs              # HTTP 助手（WebFetch）
│       └── schema.rs            # JSON Schema 验证
├── examples/
│   ├── basic_mcp.rs             # 基础 MCP 连接示例
│   └── tool_registry.rs         # 工具注册示例
└── tests/
    ├── integration_tests.rs     # 集成测试
    └── unit_tests.rs            # 单元测试
```

---

## 第二部分：实现路线图

### Phase 1: MVP (2-3 周)

#### 目标
- MCP Stdio 客户端基础实现
- Tool 接口和简单工具注册
- 内置工具：WebFetch（基础版）、WebSearch（基础版）

#### 1.1 MCP Stdio 传输 (3-5 天)

**关键文件**:
- `mcp/transport.rs` - Stdio 传输实现
- `mcp/types.rs` - JSONRPC 和 MCP 消息类型
- `mcp/client.rs` - 连接管理

**实现步骤**:

```rust
// 1. 定义 JSONRPC 消息结构
#[derive(Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,           // "2.0"
    pub id: u64,
    pub method: String,            // "initialize", "tools/list", "tools/call"
    pub params: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

// 2. 实现 Stdio 传输
pub struct StdioTransport {
    child: tokio::process::Child,
    stdin: tokio::io::BufWriter<...>,
    stdout: tokio::io::BufReader<...>,
    request_id_counter: Arc<AtomicU64>,
}

impl StdioTransport {
    pub async fn new(command: &str, args: &[String]) -> Result<Self> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        
        // 初始化并读取 Initialize 响应
        // ...
    }
    
    pub async fn send_request(&mut self, method: &str, params: serde_json::Value) 
        -> Result<serde_json::Value> {
        let id = self.request_id_counter.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };
        
        // 发送请求
        let json = serde_json::to_string(&request)?;
        self.stdin.write_all(json.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        
        // 接收响应
        let response = self.read_response(id).await?;
        Ok(response.result.ok_or(/* 错误 */)?)
    }
    
    async fn read_response(&mut self, expected_id: u64) -> Result<JsonRpcResponse> {
        // 读取行，反序列化为 JsonRpcResponse
        // 处理多行或错误
    }
}

// 3. MCP Client 包装
pub struct McpClient {
    transport: StdioTransport,
}

impl McpClient {
    pub async fn initialize(&mut self, client_info: ClientInfo) -> Result<ServerInfo> {
        let result = self.transport.send_request("initialize", serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {...},
            "clientInfo": client_info,
        })).await?;
        
        Ok(serde_json::from_value(result)?)
    }
    
    pub async fn list_tools(&mut self) -> Result<Vec<Tool>> {
        let result = self.transport.send_request("tools/list", serde_json::json!({}))
            .await?;
        
        Ok(serde_json::from_value(result)?)
    }
    
    pub async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) 
        -> Result<ToolResult> {
        let result = self.transport.send_request("tools/call", serde_json::json!({
            "name": name,
            "arguments": arguments,
        })).await?;
        
        Ok(serde_json::from_value(result)?)
    }
}
```

**验证指标**:
- [ ] Stdio 进程启动成功
- [ ] Initialize 握手成功
- [ ] 能列出工具列表
- [ ] 能调用工具并获得结果

#### 1.2 Tool 接口与工具注册 (2-3 天)

**关键文件**:
- `tools/interface.rs` - Tool trait 定义
- `tools/registry.rs` - 工具注册表
- `tools/permission.rs` - 权限系统

**实现步骤**:

```rust
// 1. 定义 Tool trait
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    
    fn description(&self) -> &str;
    
    fn input_schema(&self) -> JsonSchema;
    
    fn output_schema(&self) -> JsonSchema;
    
    async fn call(&self, input: serde_json::Value, context: &ToolContext) 
        -> Result<serde_json::Value>;
    
    async fn check_permissions(&self, input: &serde_json::Value, context: &ToolContext) 
        -> Result<PermissionResult> {
        Ok(PermissionResult::Allow)  // 默认允许
    }
    
    fn is_readonly(&self) -> bool { true }
    
    fn is_concurrency_safe(&self) -> bool { true }
}

// 2. MCP 工具适配器
pub struct McpTool {
    mcp_client: Arc<Mutex<McpClient>>,
    server_name: String,
    tool_info: McpToolInfo,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        // "mcp__<server>__<tool>"
    }
    
    async fn call(&self, input: serde_json::Value, _context: &ToolContext) 
        -> Result<serde_json::Value> {
        let mut client = self.mcp_client.lock().await;
        client.call_tool(&self.tool_info.name, input).await.map(|r| r.content)
    }
}

// 3. 工具注册表
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }
    
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }
    
    pub async fn register_mcp_tools(&mut self, client: Arc<Mutex<McpClient>>, server_name: &str) 
        -> Result<()> {
        let mut mcp = client.lock().await;
        let tools = mcp.list_tools().await?;
        
        for tool_info in tools {
            let mcp_tool = McpTool {
                mcp_client: client.clone(),
                server_name: server_name.to_string(),
                tool_info,
            };
            self.register(Arc::new(mcp_tool));
        }
        
        Ok(())
    }
    
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }
    
    pub fn list(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }
}

// 4. 权限系统
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionBehavior {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "deny")]
    Deny,
    #[serde(rename = "ask")]
    Ask,
}

#[derive(Debug, Clone)]
pub struct PermissionResult {
    pub behavior: PermissionBehavior,
    pub message: Option<String>,
}
```

**验证指标**:
- [ ] Tool trait 定义完整
- [ ] 工具注册表能注册/检索工具
- [ ] MCP 工具成功适配到 Tool 接口

#### 1.3 WebFetch 工具（基础版） (2-3 天)

**关键文件**:
- `tools/builtin/web_fetch.rs`
- `utils/http.rs`

**实现步骤**:

```rust
// 1. HTTP 获取（使用 reqwest）
pub struct WebFetchTool {
    http_client: reqwest::Client,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    
    fn description(&self) -> &str { "Fetch content from a URL and convert to markdown" }
    
    fn input_schema(&self) -> JsonSchema {
        // {"type": "object", "properties": {"url": {...}, "prompt": {...}}, ...}
    }
    
    async fn call(&self, input: serde_json::Value, _context: &ToolContext) 
        -> Result<serde_json::Value> {
        let url: String = input["url"].as_str().ok_or(...)?.to_string();
        let prompt: String = input["prompt"].as_str().ok_or(...)?.to_string();
        
        // 1. 获取 HTTP 内容
        let response = self.http_client.get(&url)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        
        let status = response.status();
        let content_type = response.headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("text/plain");
        
        let body = response.bytes().await?;
        
        // 2. HTML 转 Markdown（第一版：简单文本提取）
        let markdown = if content_type.contains("text/html") {
            // Phase 2: 集成 html2text 或 scraper
            extract_text_from_html(&String::from_utf8_lossy(&body))?
        } else {
            String::from_utf8(body.to_vec())?
        };
        
        // 3. 返回结果
        Ok(serde_json::json!({
            "bytes": body.len(),
            "code": status.as_u16(),
            "codeText": status.canonical_reason().unwrap_or("Unknown"),
            "result": markdown,
            "durationMs": 0,  // Phase 2: 加入计时
            "url": url,
        }))
    }
}

// 2. 简单 HTML 文本提取
fn extract_text_from_html(html: &str) -> Result<String> {
    // Phase 1: 使用正则表达式移除标签
    // Phase 2: 使用 scraper 或 html5ever crate
    let text = html
        .replace("</p>", "\n")
        .replace("</br>", "\n")
        .replace("<br>", "\n");
    
    let re = regex::Regex::new(r"<[^>]+>")?;
    let text = re.replace_all(&text, "").to_string();
    
    Ok(text.trim().to_string())
}
```

**验证指标**:
- [ ] 能成功获取 HTTP 内容
- [ ] 能正确处理 HTML 和 plain text
- [ ] 返回格式符合期望

#### 1.4 WebSearch 工具（基础版） (2-3 天)

**关键文件**:
- `tools/builtin/web_search.rs`

**实现步骤**:

```rust
// Phase 1: 暂时使用 Bing/Google 搜索 API 或 mock 实现
pub struct WebSearchTool {
    http_client: reqwest::Client,
    // 在 Phase 2 集成 Claude API
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    
    fn description(&self) -> &str { "Search the web for information" }
    
    fn input_schema(&self) -> JsonSchema {
        // {"type": "object", "properties": {"query": {...}, ...}, ...}
    }
    
    async fn call(&self, input: serde_json::Value, _context: &ToolContext) 
        -> Result<serde_json::Value> {
        let query: String = input["query"].as_str().ok_or(...)?.to_string();
        let allowed_domains: Option<Vec<String>> = input["allowed_domains"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());
        
        // Phase 1: Mock 实现
        let results = mock_search_results(&query)?;
        
        // Phase 2: 实现真实搜索
        // let results = perform_web_search(&query, allowed_domains).await?;
        
        Ok(serde_json::json!({
            "query": query,
            "results": results,
            "durationSeconds": 1.0,
        }))
    }
}

fn mock_search_results(query: &str) -> Result<Vec<serde_json::Value>> {
    // 返回 mock 搜索结果，用于测试
    Ok(vec![
        serde_json::json!({
            "title": format!("Result for: {}", query),
            "url": "https://example.com",
        }),
    ])
}
```

**验证指标**:
- [ ] WebSearch 工具能注册
- [ ] 能返回正确格式的搜索结果

### Phase 2: 完整功能 (2-3 周)

#### 目标
- 完整 MCP 功能（错误处理、超时、重试）
- WebFetch 完整实现（HTML 解析、缓存、域名检查）
- WebSearch 真实集成（Claude API）
- 错误处理和日志系统

#### 2.1 增强 MCP 客户端

**关键改进**:
```rust
// 超时控制
pub async fn call_tool_with_timeout(&mut self, name: &str, args: serde_json::Value, timeout: Duration) 
    -> Result<ToolResult> {
    tokio::time::timeout(timeout, self.call_tool(name, args)).await?
}

// 自动重连
pub struct McpClientWithReconnect {
    config: McpServerConfig,
    client: Option<McpClient>,
    reconnect_attempts: u32,
}

impl McpClientWithReconnect {
    pub async fn ensure_connected(&mut self) -> Result<&mut McpClient> {
        if self.client.is_none() || self.is_connection_dead() {
            self.client = Some(McpClient::new(&self.config).await?);
        }
        Ok(self.client.as_mut().unwrap())
    }
}

// 流式工具调用（支持进度报告）
pub async fn call_tool_streaming(&mut self, ...) 
    -> Result<impl Stream<Item = ToolProgress>> {
    // 支持 server_tool_result 的分块交付
}
```

#### 2.2 增强 WebFetch

**关键改进**:
```rust
// HTML 解析（使用 scraper crate）
use scraper::{Html, Selector};

fn parse_html_to_markdown(html: &str) -> Result<String> {
    let document = Html::parse_document(html);
    
    // 提取主要内容
    let body_selector = Selector::parse("body").unwrap();
    let body = document.select(&body_selector).next().unwrap();
    
    // 转换为 Markdown
    html_to_markdown(body.inner_html())
}

// 缓存实现（使用 moka crate）
pub struct WebFetchCache {
    cache: moka::future::Cache<String, CacheEntry>,
}

impl WebFetchCache {
    pub async fn get_or_fetch(&self, url: &str, fetcher: impl Fn() -> ... ) 
        -> Result<CacheEntry> {
        self.cache.get_or_insert_with(url.to_string(), async {
            fetcher().await
        }).await
    }
}

// 域名检查
pub async fn check_domain_blocklist(domain: &str) -> Result<bool> {
    // 调用 Anthropic API 检查域名
    let url = format!("https://api.anthropic.com/api/web/domain_info?domain={}", domain);
    let response = reqwest::Client::new().get(&url).send().await?;
    let data: serde_json::Value = response.json().await?;
    Ok(data["can_fetch"].as_bool().unwrap_or(false))
}
```

#### 2.3 WebSearch 真实集成

**关键改进**:
```rust
// 集成 Claude API
pub struct WebSearchWithClaudeApi {
    client: anthropic::Client,
}

impl WebSearchWithClaudeApi {
    pub async fn search(&self, query: &str, allowed_domains: Option<Vec<String>>) 
        -> Result<SearchResults> {
        // 使用 Claude Messages API + web_search tool
        let response = self.client.messages()
            .create(MessageRequest {
                model: "claude-opus-4-20250805",
                max_tokens: 1024,
                tools: vec![
                    Tool::WebSearch(WebSearchTool {
                        allowed_domains,
                        blocked_domains: None,
                    })
                ],
                // ...
            })
            .await?;
        
        // 提取搜索结果
        parse_search_results_from_response(response)
    }
}
```

#### 2.4 错误处理和日志

**关键改进**:
```rust
// 自定义错误类型
#[derive(Debug)]
pub enum SageError {
    McpError(String),
    HttpError(reqwest::Error),
    SerializationError(serde_json::Error),
    TimeoutError,
    PermissionDenied(String),
    InvalidInput(String),
}

impl Display for SageError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::McpError(msg) => write!(f, "MCP Error: {}", msg),
            Self::TimeoutError => write!(f, "Operation timeout"),
            // ...
        }
    }
}

// 集成日志
pub fn initialize_logging() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
}
```

### Phase 3: 优化与完善 (1-2 周)

#### 目标
- 性能优化（考虑枚举或宏替代 trait 对象）
- 完整的集成测试
- 文档和示例

#### 3.1 性能优化

```rust
// Phase 2 MVP：使用 trait 对象
pub type ToolBox = Box<dyn Tool>;

// Phase 3 优化：使用枚举
pub enum ToolEnum {
    WebFetch(WebFetchTool),
    WebSearch(WebSearchTool),
    Mcp(McpTool),
}

impl Tool for ToolEnum {
    fn name(&self) -> &str {
        match self {
            Self::WebFetch(t) => t.name(),
            Self::WebSearch(t) => t.name(),
            Self::Mcp(t) => t.name(),
        }
    }
    // ...
}
```

#### 3.2 完整测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_mcp_connection() {
        // 测试 MCP 连接建立
    }
    
    #[tokio::test]
    async fn test_tool_registry() {
        // 测试工具注册和检索
    }
    
    #[tokio::test]
    async fn test_web_fetch() {
        // 测试 WebFetch 工具
    }
    
    #[tokio::test]
    async fn test_permissions() {
        // 测试权限检查
    }
}
```

---

## 第三部分：与 TypeScript 实现的映射

### 类型映射

| TypeScript | Rust | 注意事项 |
|-----------|------|--------|
| `interface Tool` | `trait Tool` | 添加 `#[async_trait]` |
| `Promise<T>` | `impl Future<Output=T>` / `async` | 使用 tokio 异步 |
| `Record<string, any>` | `serde_json::Value` / `HashMap<String, T>` | 根据场景选择 |
| `z.strictObject(...)` | `serde(deny_unknown_fields)` | 使用 serde attributes |
| `Optional[T]` | `Option<T>` | Rust 惯例 |
| `Array<T>` | `Vec<T>` / `&[T]` | 根据所有权选择 |
| `enum` (union) | `enum` | Rust 有完整枚举支持 |
| `LRUCache` | `lru::LruCache` 或 `moka::Cache` | Phase 2 升级到 moka |

### 函数映射

| TypeScript | Rust | 文件 |
|-----------|------|------|
| `connectToServer()` | `McpClient::new()` | `mcp/client.rs` |
| `fetchToolsForClient()` | `McpClient::list_tools()` | `mcp/client.rs` |
| `callMCPTool()` | `Tool::call()` | `tools/interface.rs` |
| `buildTool()` | 工厂函数或构造函数 | `tools/builtin/*.rs` |
| `getURLMarkdownContent()` | `WebFetchTool::call()` | `tools/builtin/web_fetch.rs` |

---

## 第四部分：外部依赖

### 必需依赖

```toml
[dependencies]
# 异步运行时
tokio = { version = "1", features = ["full"] }

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# HTTP
reqwest = { version = "0.11", features = ["json"] }

# 异步 trait
async-trait = "0.1"

# 正则表达式（HTML 提取）
regex = "1"

# HTTP 客户端辅助
url = "2"

# 日志
tracing = "0.1"
tracing-subscriber = "0.3"

# 错误处理
thiserror = "1"

# 类型增强
derive_builder = "0.12"
```

### 可选依赖（Phase 2+）

```toml
[dependencies.phase2]
# HTML 解析
scraper = "0.17"
select = "0.6"

# 高性能缓存
moka = { version = "0.12", features = ["future"] }

# Markdown 生成
html2text = "0.2"

# JSON Schema 验证
jsonschema = "0.17"

# Claude API
anthropic-sdk = "0.1"  # 如果存在官方 SDK
```

---

## 第五部分：验收标准

### MVP 验收标准 (Phase 1)

- [ ] MCP Stdio 客户端能成功连接到 MCP 服务器
- [ ] 能列出远程 MCP 工具
- [ ] 能调用远程工具并获得结果
- [ ] 内置 WebFetch 工具能获取 HTTP 内容并转换为文本
- [ ] 内置 WebSearch 工具能返回 mock 搜索结果
- [ ] Tool registry 能注册和检索工具
- [ ] 权限系统能进行基本的权限检查
- [ ] 所有单元测试通过
- [ ] 能在 Sage 架构中作为 Plugin 运行

### 完整功能验收标准 (Phase 2)

- [ ] MCP 客户端支持超时控制和自动重连
- [ ] WebFetch 支持 HTML 解析到 Markdown
- [ ] WebFetch 支持 LRU 缓存和 TTL
- [ ] WebFetch 支持域名检查
- [ ] WebSearch 集成 Claude API 进行真实搜索
- [ ] 完整的错误处理和日志系统
- [ ] 集成测试覆盖主要流程
- [ ] 文档完整

### 优化完善验收标准 (Phase 3)

- [ ] 考虑了 trait 对象与枚举的性能权衡
- [ ] 完整的性能基准测试
- [ ] 并发性能测试
- [ ] 内存使用优化
- [ ] 详细的 API 文档
- [ ] 多个完整的使用示例

---

## 第六部分：风险与缓解

### 技术风险

| 风险 | 影响 | 缓解措施 |
|-----|------|--------|
| MCP 协议变更 | 实现失效 | 定期测试与官方实现同步 |
| Stdio 进程通信延迟 | 性能下降 | Phase 2+ 评估 WebSocket/HTTP 替代 |
| HTML 解析复杂性 | 开发周期延长 | 使用成熟库（scraper），测试覆盖 |
| 缓存一致性 | 数据错误 | 完整的单元测试，TTL 机制 |

### 时间风险

| 风险 | 缓解措施 |
|-----|--------|
| 依赖库 API 变化 | 早期集成测试，版本锁定 |
| 学习曲线 | 代码审查，知识分享会 |
| 集成困难 | 模块化设计，提前集成测试 |

---

## 第七部分：建议与后续工作

### 立即开始

1. 创建 Cargo 项目结构
2. 研究和选择 MCP stdio 实现方案
3. 启动 Phase 1 MCP 客户端开发
4. 建立开发和测试环境

### Phase 2 规划

1. 性能测试和分析
2. WebFetch HTML 解析库选择与集成
3. WebSearch Claude API 集成方案验证
4. 错误处理和恢复机制设计

### Phase 3 规划

1. 性能优化方案评估
2. 文档生成工具链
3. 发布和版本管理流程

---

## 附录：CLI 参考示例

### 创建 Cargo 项目

```bash
cargo new sage-rune --lib
cd sage-rune
cargo add tokio serde serde_json async-trait
```

### 构建和测试

```bash
cargo build                 # 调试构建
cargo build --release      # 发布构建
cargo test                 # 运行所有测试
cargo test -- --nocapture # 显示输出
cargo doc --open          # 生成并打开文档
```

### 运行示例

```bash
cargo run --example basic_mcp    # 运行基础 MCP 示例
cargo run --example tool_registry # 运行工具注册示例
```

---

**文档版本**: 1.0
**最后更新**: 2026-04-21
**负责人**: Sage Research Agent
