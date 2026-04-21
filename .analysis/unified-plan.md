# Sage v0.2.0 Hooks 系统 Rust 实现计划

## 概述

本计划描述如何在 Sage Rust 运行时中实现 Claude Code 的 hooks 系统。Hooks 是在会话生命周期的特定点执行任意代码的扩展机制，支持：
- Shell 命令
- LLM-based 评估（Prompt hooks）
- HTTP 远程调用
- Agentic verification

## 实现分阶段计划

### 第一阶段：基础数据结构和配置解析

**目标**：建立 Rust 中的 hooks 数据模型和配置加载机制。

#### 1.1 定义数据结构

**文件**：`src/hooks/mod.rs`, `src/hooks/types.rs`

```rust
// Hook 事件类型枚举
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    SubagentStop,
    PermissionDenied,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Setup,
    SubagentStart,
    Notification,
    TeammateIdle,
    TaskCreated,
    TaskCompleted,
    PreCompact,
    PostCompact,
}

// Hook 命令类型（对应 settings.json 中的配置）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HookCommand {
    Command {
        command: String,
        shell: Option<ShellType>,  // bash, powershell
        timeout: Option<u64>,      // 秒
        async_: Option<bool>,
        async_rewake: Option<bool>,
        once: Option<bool>,
        status_message: Option<String>,
        #[serde(rename = "if")]
        if_condition: Option<String>,
    },
    Prompt {
        prompt: String,
        model: Option<String>,
        timeout: Option<u64>,
        once: Option<bool>,
        status_message: Option<String>,
        #[serde(rename = "if")]
        if_condition: Option<String>,
    },
    Http {
        url: String,
        headers: Option<HashMap<String, String>>,
        allowed_env_vars: Option<Vec<String>>,
        timeout: Option<u64>,
        once: Option<bool>,
        status_message: Option<String>,
        #[serde(rename = "if")]
        if_condition: Option<String>,
    },
    Agent {
        prompt: String,
        model: Option<String>,
        timeout: Option<u64>,
        once: Option<bool>,
        status_message: Option<String>,
        #[serde(rename = "if")]
        if_condition: Option<String>,
    },
}

// Hook 匹配器配置
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookMatcher {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,  // 工具名或权限规则
    pub hooks: Vec<HookCommand>,
}

// Hooks 配置（从 settings.json 加载）
pub type HooksConfig = HashMap<HookEvent, Vec<HookMatcher>>;

// Hook 输入（传递给 hook 进程的参数）
#[derive(Serialize)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub hook_event_name: String,
    #[serde(flatten)]
    pub event_specific: serde_json::Value,  // PreToolUse, PostToolUse 等特定字段
}

// Hook 输出（解析 hook 返回的 JSON）
#[derive(Debug, Deserialize)]
pub struct HookJsonOutput {
    #[serde(skip)]
    pub continue_: Option<bool>,  // continue 是保留字，需要重映射
    pub suppress_output: Option<bool>,
    pub stop_reason: Option<String>,
    pub decision: Option<HookDecision>,  // 'approve' | 'block'
    pub reason: Option<String>,
    pub system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
    // 异步 hooks
    #[serde(skip)]
    pub async_: Option<bool>,  // 重映射 'async'
    pub async_timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookDecision {
    Approve,
    Block,
}

// Hook 特定输出（PreToolUse, PostToolUse 等）
#[derive(Debug, Deserialize)]
#[serde(tag = "hookEventName")]
pub enum HookSpecificOutput {
    PreToolUse {
        permission_decision: Option<PermissionDecision>,
        permission_decision_reason: Option<String>,
        updated_input: Option<serde_json::Value>,
        additional_context: Option<String>,
    },
    PostToolUse {
        additional_context: Option<String>,
        updated_mcp_tool_output: Option<serde_json::Value>,
    },
    PostToolUseFailure {
        additional_context: Option<String>,
        retry: Option<bool>,
    },
    // ... 其他 hook 类型
}

// Hook 执行结果
pub struct HookResult {
    pub outcome: HookOutcome,
    pub message: Option<String>,
    pub system_message: Option<String>,
    pub blocking_error: Option<HookBlockingError>,
    pub updated_input: Option<serde_json::Value>,
    pub additional_context: Option<String>,
    pub stop_reason: Option<String>,
    pub permission_behavior: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOutcome {
    Success,
    Blocking,
    NonBlockingError,
    Cancelled,
}

pub struct HookBlockingError {
    pub message: String,
    pub command: String,
}
```

#### 1.2 配置加载

**文件**：`src/hooks/config.rs`

```rust
pub struct HooksConfig {
    hooks: HashMap<HookEvent, Vec<HookMatcher>>,
    trust_required: bool,
    has_trust: bool,  // 从系统检查
}

impl HooksConfig {
    /// 从 settings.json 加载 hooks 配置
    pub fn load_from_settings(settings_path: &Path) -> Result<Self> {
        // 1. 读取 JSON
        // 2. 验证 schema（Zod 对应的 Rust validation）
        // 3. 构建 HooksConfig
        // 4. 检查信任状态
    }

    /// 获取特定事件的 hooks
    pub fn get_hooks_for_event(&self, event: HookEvent) -> Vec<&HookMatcher> {
        self.hooks.get(&event).map(|v| v.iter().collect()).unwrap_or_default()
    }

    /// 应用匹配器和条件过滤
    pub fn filter_hooks(
        &self,
        event: HookEvent,
        tool_name: Option<&str>,
        tool_input: Option<&serde_json::Value>,
    ) -> Vec<&HookCommand> {
        // 1. 获取事件对应的 hooks
        // 2. 按 matcher 字符串过滤（如果适用）
        // 3. 按 if 条件过滤（权限规则语法）
        // 4. 返回匹配的 hook 列表
    }
}
```

### 第二阶段：Hook 执行引擎

**目标**：实现 hook 进程执行、通信、结果解析的核心逻辑。

#### 2.1 进程执行

**文件**：`src/hooks/executor.rs`

```rust
pub struct HookExecutor {
    config: HooksConfig,
    session_context: SessionContext,  // session_id, cwd, transcript_path
    timeout_ms: u64,
}

impl HookExecutor {
    pub async fn execute_hook(
        &self,
        hook: &HookCommand,
        hook_input: &HookInput,
        signal: Option<&CancellationSignal>,
    ) -> Result<HookResult> {
        match hook {
            HookCommand::Command { command, shell, timeout, async_, .. } => {
                self.execute_command_hook(command, shell.as_ref(), timeout, hook_input, signal).await
            }
            HookCommand::Prompt { prompt, model, .. } => {
                self.execute_prompt_hook(prompt, model.as_ref(), hook_input, signal).await
            }
            HookCommand::Http { url, headers, .. } => {
                self.execute_http_hook(url, headers.as_ref(), hook_input, signal).await
            }
            HookCommand::Agent { prompt, model, .. } => {
                self.execute_agent_hook(prompt, model.as_ref(), hook_input, signal).await
            }
        }
    }

    /// 执行 shell 命令 hook
    async fn execute_command_hook(
        &self,
        command: &str,
        shell: Option<&ShellType>,
        timeout: &Option<u64>,
        hook_input: &HookInput,
        signal: Option<&CancellationSignal>,
    ) -> Result<HookResult> {
        // 1. 序列化 hook_input 为 JSON
        let input_json = serde_json::to_string(&hook_input)?;

        // 2. 选择 shell（bash 或 powershell）
        let shell_cmd = self.resolve_shell(shell)?;

        // 3. 通过环境变量或 stdin 传递 hook_input
        // 选项 A：HOOK_INPUT_JSON 环境变量
        // 选项 B：stdin（通过 heredoc）

        // 4. 用 tokio::process::Command 执行
        let mut child = tokio::process::Command::new(shell_cmd)
            .arg("-c")
            .arg(command)
            .env("HOOK_INPUT_JSON", input_json)
            .env("CLAUDE_HOOK_ID", uuid::Uuid::new_v4().to_string())
            .env("CLAUDE_HOOK_EVENT", &hook_input.hook_event_name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        // 5. 超时控制和信号处理
        let timeout_duration = Duration::from_secs(*timeout.unwrap_or(10));
        let result = tokio::time::timeout(timeout_duration, child.wait_with_output()).await;

        // 6. 解析 stdout/stderr 和 exit code
        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);
                
                self.parse_hook_output(&stdout, &stderr, exit_code)
            }
            Ok(Err(e)) => Err(HookError::ExecutionFailed(e.to_string())),
            Err(_) => {
                child.kill().await.ok();
                Err(HookError::Timeout)
            }
        }
    }

    /// 解析 hook 输出（JSON 或纯文本）
    fn parse_hook_output(
        &self,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> Result<HookResult> {
        let trimmed = stdout.trim();
        
        // 1. 检查是否为 JSON（以 { 开头）
        if trimmed.starts_with('{') {
            // 2. 反序列化并验证 HookJsonOutput
            match serde_json::from_str::<HookJsonOutput>(trimmed) {
                Ok(json_output) => self.process_hook_json_output(json_output, exit_code),
                Err(e) => {
                    // 验证失败，作为纯文本处理
                    Ok(HookResult {
                        outcome: HookOutcome::NonBlockingError,
                        message: Some(format!("JSON validation failed: {}", e)),
                        ..Default::default()
                    })
                }
            }
        } else {
            // 3. 纯文本处理
            Ok(HookResult {
                outcome: HookOutcome::Success,
                message: Some(stdout.to_string()),
                ..Default::default()
            })
        }
    }

    /// 处理 JSON 输出和 exit code
    fn process_hook_json_output(
        &self,
        json: HookJsonOutput,
        exit_code: i32,
    ) -> Result<HookResult> {
        let outcome = match exit_code {
            0 => HookOutcome::Success,
            1 => HookOutcome::NonBlockingError,
            2 => HookOutcome::Blocking,
            _ => HookOutcome::NonBlockingError,
        };

        // 处理 decision 字段（旧格式）或权限决策
        let blocking_error = if let Some(HookDecision::Block) = &json.decision {
            Some(HookBlockingError {
                message: json.reason.unwrap_or_else(|| "Blocked by hook".to_string()),
                command: "unknown".to_string(),
            })
        } else {
            None
        };

        Ok(HookResult {
            outcome,
            blocking_error,
            system_message: json.system_message,
            // ... 其他字段处理
        })
    }

    /// 执行 Prompt hook（调用 Claude API）
    async fn execute_prompt_hook(
        &self,
        prompt: &str,
        model: Option<&str>,
        hook_input: &HookInput,
        signal: Option<&CancellationSignal>,
    ) -> Result<HookResult> {
        // 1. 替换 $ARGUMENTS 占位符为 JSON 序列化的 hook_input
        let input_json = serde_json::to_string(&hook_input)?;
        let final_prompt = prompt.replace("$ARGUMENTS", &input_json);

        // 2. 调用 Claude API（使用提供的或默认模型）
        let client = self.create_anthropic_client()?;
        let response = client.create_message(&final_prompt, model).await?;

        // 3. 解析响应（应为 JSON）
        self.parse_hook_output(&response, "", 0)
    }

    /// 执行 HTTP hook
    async fn execute_http_hook(
        &self,
        url: &str,
        headers: Option<&HashMap<String, String>>,
        hook_input: &HookInput,
        signal: Option<&CancellationSignal>,
    ) -> Result<HookResult> {
        // 1. 序列化 hook_input
        let body = serde_json::to_string(&hook_input)?;

        // 2. 构建 HTTP 请求
        // 3. 替换 headers 中的环境变量
        let mut final_headers = headers.cloned().unwrap_or_default();
        for (_, value) in &mut final_headers {
            *value = self.interpolate_env_vars(value);
        }

        // 4. POST 请求
        let client = reqwest::Client::new();
        let response = client.post(url)
            .headers(parse_headers(&final_headers)?)
            .body(body)
            .send()
            .await?;

        // 5. 解析响应
        let response_body = response.text().await?;
        self.parse_hook_output(&response_body, "", 0)
    }
}
```

#### 2.2 Hook 事件触发

**文件**：`src/hooks/triggers.rs`

```rust
pub struct HookTriggers {
    executor: HookExecutor,
}

impl HookTriggers {
    /// PreToolUse hook — 工具调用前触发
    pub async fn execute_pre_tool_use_hooks(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_use_id: &str,
    ) -> Result<Vec<HookResult>> {
        let hook_input = HookInput {
            hook_event_name: "PreToolUse".to_string(),
            event_specific: serde_json::json!({
                "tool_name": tool_name,
                "tool_input": tool_input,
                "tool_use_id": tool_use_id,
            }),
            ..self.executor.session_context.to_base_input()
        };

        let hooks = self.executor.config.filter_hooks(
            HookEvent::PreToolUse,
            Some(tool_name),
            Some(tool_input),
        );

        let mut results = Vec::new();
        for hook in hooks {
            let result = self.executor.execute_hook(hook, &hook_input, None).await?;
            results.push(result);
        }
        Ok(results)
    }

    /// PostToolUse hook — 工具成功执行后触发
    pub async fn execute_post_tool_use_hooks(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_response: &serde_json::Value,
        tool_use_id: &str,
    ) -> Result<Vec<HookResult>> {
        // 类似 PreToolUse，但包含 tool_response
    }

    /// Stop hook — 会话停止时触发
    pub async fn execute_stop_hooks(
        &self,
        last_message: Option<&str>,
    ) -> Result<Vec<HookResult>> {
        // 不需要工具名匹配
    }

    /// 聚合多个 hooks 的结果
    pub fn aggregate_results(&self, results: Vec<HookResult>) -> AggregatedResult {
        // 1. 检查是否有阻塞错误
        // 2. 合并 additional_contexts
        // 3. 合并 updated_inputs
        // 4. 返回聚合结果
    }
}
```

### 第三阶段：权限和信任集成

**目标**：实现信任检查和权限决策集成。

#### 3.1 信任检查

**文件**：`src/hooks/trust.rs`

```rust
pub struct TrustManager {
    workspace_trusted: bool,  // 从系统查询
}

impl TrustManager {
    /// 检查是否应跳过 hook（因信任问题）
    pub fn should_skip_hook(&self, is_interactive: bool) -> bool {
        // SDK 模式（非交互）隐式信任
        if !is_interactive {
            return false;
        }
        // 交互模式需要用户信任
        !self.workspace_trusted
    }

    /// 所有 hooks 都需要信任（包括 SessionEnd）
    pub fn verify_trust_before_hook_execution(&self) -> Result<()> {
        if !self.workspace_trusted {
            return Err(HookError::TrustNotEstablished);
        }
        Ok(())
    }
}
```

#### 3.2 权限决策

**文件**：`src/hooks/permissions.rs`

```rust
pub struct PermissionIntegration {
    permission_system: Arc<PermissionSystem>,
}

impl PermissionIntegration {
    /// PreToolUse hook 可以覆盖权限决策
    pub fn apply_hook_permission_decision(
        &self,
        tool_name: &str,
        hook_decision: Option<PermissionDecision>,
    ) -> Result<PermissionBehavior> {
        match hook_decision {
            Some(PermissionDecision::Allow) => Ok(PermissionBehavior::Allow),
            Some(PermissionDecision::Deny) => Ok(PermissionBehavior::Deny),
            Some(PermissionDecision::Ask) => Ok(PermissionBehavior::Ask),
            None => {
                // 使用权限系统的决策
                self.permission_system.check_permission(tool_name)
            }
        }
    }
}
```

### 第四阶段：异步和生命周期管理

**目标**：实现异步执行、后台 hooks、会话生命周期事件。

#### 4.1 异步 Hook 执行

**文件**：`src/hooks/async_executor.rs`

```rust
pub struct AsyncHookManager {
    pending_hooks: Arc<Mutex<HashMap<String, PendingHook>>>,
}

pub struct PendingHook {
    hook_id: String,
    handle: JoinHandle<Result<HookResult>>,
    hook_event: String,
    asyncRewake: bool,
}

impl AsyncHookManager {
    /// 启动异步 hook 执行
    pub async fn spawn_async_hook(
        &self,
        hook_id: String,
        hook: HookCommand,
        input: HookInput,
        async_rewake: bool,
    ) -> Result<()> {
        let handle = tokio::spawn(async move {
            // 执行 hook
            executor.execute_hook(&hook, &input, None).await
        });

        let pending = PendingHook {
            hook_id: hook_id.clone(),
            handle,
            hook_event: input.hook_event_name.clone(),
            asyncRewake: async_rewake,
        };

        self.pending_hooks.lock().unwrap().insert(hook_id, pending);
        Ok(())
    }

    /// 检查完成的异步 hooks
    pub async fn check_completed_hooks(&self) -> Result<Vec<HookCompletion>> {
        // 1. 遍历 pending_hooks
        // 2. 检查哪些已完成
        // 3. 如果 asyncRewake && exit_code == 2，生成唤醒通知
        // 4. 返回完成的 hooks
    }

    /// SessionEnd 时等待所有 hooks（带超时）
    pub async fn wait_for_all_hooks(&self, timeout_ms: u64) -> Result<()> {
        // 1. 设置超时
        // 2. 等待所有 pending hooks 完成
        // 3. 杀死超时的 hooks
    }
}
```

#### 4.2 会话生命周期事件

**文件**：`src/hooks/lifecycle.rs`

```rust
pub struct HookLifecycleManager {
    executor: HookExecutor,
    async_manager: AsyncHookManager,
}

impl HookLifecycleManager {
    /// SessionStart hook — 会话开始时触发
    pub async fn trigger_session_start(&self) -> Result<()> {
        // 触发 SessionStart hooks（可以初始化或修改初始 prompt）
    }

    /// SessionEnd hook — 会话结束时触发（带紧凑的 1.5s 超时）
    pub async fn trigger_session_end(&self) -> Result<()> {
        // 1. 获取 SessionEnd hooks
        // 2. 执行（带短超时）
        // 3. 等待 asyncRewake hooks 完成
    }

    /// Setup hook — 设置阶段（初始化）
    pub async fn trigger_setup(&self) -> Result<()> {
        // 执行初始化 hooks
    }
}
```

### 第五阶段：测试和集成

**目标**：实现单元测试、集成测试和端到端测试。

#### 5.1 单元测试

**文件**：`tests/hooks_unit_tests.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_hook_json_output() {
        // 测试 JSON 反序列化和验证
    }

    #[test]
    fn test_hook_matching() {
        // 测试 matcher 字符串匹配
        // 测试权限规则过滤
    }

    #[tokio::test]
    async fn test_command_hook_execution() {
        // 测试 shell 命令执行
        // 测试 timeout
        // 测试 exit codes
    }

    #[tokio::test]
    async fn test_trust_verification() {
        // 测试信任检查逻辑
    }
}
```

#### 5.2 集成测试

**文件**：`tests/hooks_integration_tests.rs`

```rust
#[tokio::test]
async fn test_pre_tool_use_hook_blocking() {
    // 设置 hook 配置
    // 执行工具调用
    // 验证被 hook 拦截
}

#[tokio::test]
async fn test_post_tool_use_hook_modifies_output() {
    // 执行工具调用
    // 触发 PostToolUse hook
    // 验证输出被修改
}

#[tokio::test]
async fn test_async_hook_rewake() {
    // 配置异步 hook with asyncRewake
    // 让 hook 以 exit code 2 退出
    // 验证模型被唤醒
}
```

### 第六阶段：文档和示例

**目标**：提供配置示例和开发者文档。

#### 6.1 用户文档

**文件**：`./.claude/HOOKS.md`（或项目内文档）

```markdown
# Sage Hooks 配置指南

## 快速开始

在 `.claude/settings.json` 中配置 hooks：

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Write",
        "hooks": [
          {
            "type": "command",
            "command": "echo 'Checking write: $tool_name' && exit 0"
          }
        ]
      }
    ]
  }
}
```

## 支持的 Hook 类型
- Command（shell）
- Prompt（LLM）
- HTTP（远程）
- Agent（agentic）

...（详细文档）
```

#### 6.2 开发者指南

API 文档和示例代码。

## 关键实现细节

### 错误处理

1. **Hook 执行失败**：非阻塞错误记录
2. **超时**：杀死进程并返回超时错误
3. **JSON 验证失败**：作为纯文本处理（降级）
4. **信任检查失败**：跳过 hook 执行

### 性能考虑

1. **Hook 缓存**：配置在启动时加载，不频繁重新加载
2. **Matcher 优化**：预编译权限规则
3. **并发执行**：多个 hooks 可以并发执行（对于同一事件）
4. **Async Hooks**：不阻塞主流程

### 安全考虑

1. **信任验证**：所有 hooks 需要工作区信任
2. **环境隔离**：hooks 在独立进程中运行
3. **权限检查**：支持权限规则过滤
4. **环境变量插值**：明确白名单

## 时间表建议

- **第一阶段**：1-2 天（数据结构和配置）
- **第二阶段**：2-3 天（执行引擎）
- **第三阶段**：1-2 天（权限和信任）
- **第四阶段**：2 天（异步和生命周期）
- **第五阶段**：1-2 天（测试）
- **第六阶段**：1 天（文档）

**总计**：约 8-12 天

## 成功标准

✅ 所有 hook 类型可执行（command, prompt, HTTP, agent）
✅ PreToolUse hooks 可以拦截和修改工具输入
✅ PostToolUse hooks 可以修改输出
✅ Stop hooks 在会话结束时触发
✅ 异步和 asyncRewake 支持工作
✅ 信任检查正确实施
✅ 权限决策集成正确
✅ 完整的单元和集成测试覆盖
✅ 文档和配置示例完整
