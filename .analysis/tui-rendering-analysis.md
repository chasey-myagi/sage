# TUI 渲染系统分析 - CC 实现研究

## 概览

Claude Code (CC) 的 TUI 渲染系统采用 React/Ink 框架，通过高度优化的组件库和滚动处理实现了流畅的终端 UI。Sage v0.2.0 的 Rust 实现需要从 CC 中借鉴核心概念，但不依赖 React，而是使用 ratatui 进行渲染。

---

## 一、CC 的 TUI 渲染架构

### 1.1 总体结构
- **REPL.tsx** - 主屏幕容器，管理消息历史、权限请求、输入处理、工具确认
- **VirtualMessageList.tsx** - 虚拟列表容器，处理消息渲染、搜索、导航
- **ScrollBox.tsx** - 通用滚动容器，提供低开销的滚动 API
- **ScrollKeybindingHandler.tsx** - 键盘和鼠标滚轮处理，包含智能加速算法
- **Message.tsx** - 消息类型分派，根据消息类型选择渲染器
- **MessageRow.tsx** - 单条消息行的布局容器

### 1.2 三层设计
1. **消息层** - 消息类型（User/Assistant/System/Tool/Attachment）
2. **列表层** - 虚拟化、搜索、导航、高度缓存
3. **滚动层** - 低级 DOM 操作、viewport 裁剪、sticky 模式

---

## 二、ScrollBox 组件 - 核心滚动实现

### 2.1 关键特性

#### 命令式 API（不走 React state）
```typescript
scrollTo(y: number)           // 绝对位置
scrollBy(dy: number)          // 相对滚动（累积）
scrollToBottom()              // 粘性底部
scrollToElement(el, offset)   // 滚动到元素（render 时解析位置）
```

#### Sticky Scroll（自动跟随新内容）
- 属性：`el.stickyScroll ?? attributes['stickyScroll']`
- 当内容增长时自动保持底部可见
- 手动 `scrollTo/scrollBy` 会中断粘性
- 用于 agent 流式输出场景

#### Viewport 裁剪与渲染优化
- 只渲染 `[scrollTop, scrollTop + viewportHeight]` 范围内的子元素
- 内容通过 `transform: translateY(-scrollTop)` 平移和裁剪
- 避免渲染屏幕外的数千条消息

#### 微任务批处理
- `scrollBy` 调用先累积到 `el.pendingScrollDelta`
- 通过 `queueMicrotask` 在一个 input batch 内合并多次滚动
- 单次 render 而非每个 wheel 事件都渲染

#### 位置锚点（render-time 解析）
```typescript
el.scrollAnchor = { el: targetElement, offset: 10 }
// render 阶段读取 targetElement.yogaNode.getComputedTop()
// 保证读取值与 Yoga 计算同步（不存在陈旧数据）
```

### 2.2 DOM 属性扩展
- `scrollTop` - 当前滚动位置
- `pendingScrollDelta` - 待处理的相对滚动增量
- `scrollAnchor` - 位置锚点对象
- `stickyScroll` - 布尔型粘性标志（DOM 属性优先级）
- `scrollClampMin/Max` - render 时强制的边界夹钳值

### 2.3 订阅机制
```typescript
ref.current?.subscribe(() => {
  // 在 scrollTo/scrollBy/scrollToBottom 之后触发
  // （sticky 更新由 render 阶段处理，不触发此回调）
})
```

---

## 三、VirtualMessageList - 虚拟滚动与消息列表

### 3.1 虚拟化策略
- 只挂载当前可见范围 ±缓冲区的消息 DOM
- 通过 `useVirtualScroll` hook 计算可见范围
- 高度缓存按 column 宽度分层（宽度改变时失效）

### 3.2 搜索与导航
- **搜索文本提取** - 预先计算并缓存每条消息的可搜索文本
- **增量搜索** - 按下 `/` 时记录锚点，输入预览，Enter 确认
- **n/N 导航** - 在匹配间跳转，保持锚点稳定

### 3.3 高度测量
- Yoga layout 计算每条消息的高度
- 使用 `useVirtualScroll` 的 `getFreshScrollHeight()`（直接读取 Yoga）
- 宽度变化时清除缓存，重新测量

### 3.4 Sticky Prompt
- 用户粘贴的长 prompt 显示在顶部，可以通过 click 隐藏 padding
- 通过 `ScrollChromeContext` 与布局通信（不是 callback）

### 3.5 消息选择与点击
- `onItemClick` - toggle 每条消息的详细模式
- `isItemClickable` - 判断消息是否响应点击（某些类型如纯文本不响应）
- `isItemExpanded` - 持久灰色背景表示展开状态

---

## 四、ScrollKeybindingHandler - 键盘和滚轮处理

### 4.1 核心输入处理
- **j/k** - 单行上下滚动
- **g/G** - 跳到顶部/底部（modal 模式）
- **Ctrl+U/D/B/F** - 上/下/上翻页/下翻页（modal 模式）
- **Wheel** - 鼠标滚轮（自适应加速）
- **Arrow up/down** - 终端原生支持

### 4.2 Wheel 加速算法（高度复杂）

#### 原生终端路径（Ghostty 等）
- 基础：每个 SGR wheel 事件 = 1 行意图
- 加速：事件到达速度快时，乘数线性增长
- 窗口：40ms 内的事件视为连续滚动
- 最大加速度：6 倍

#### xterm.js / VS Code 集成终端路径
- 基础：每个事件 = 1 行（不预乘）
- 加速：指数衰减曲线，半衰期 150ms
- 动量：`momentum = 0.5^(gap/halflife)`
- 稳态：`1 + step * m / (1-m)`，上限 3-6（gap 相关）

#### 鼠标编码器反弹检测
- 损坏/廉价光学编码器会在快速旋转时产生反向脉冲（28% 反向率观察）
- 轨迹板不产生反弹（0% 反弹率）
- 检测到反弹 = 真实鼠标，启用衰减曲线
- 反弹返回必须在 200ms 内（否则视为独立事件）

#### 设备切换检测
- 空闲 gap > 1500ms 时重置为精确模式
- 允许用户在鼠标和轨迹板间切换而无缝衔接

### 4.3 搜索上下文维护
- `onScroll` 回调触发时判断：sticky state 和滚动位置
- 手动滚动（j/k/wheel）会取消搜索突出显示（但保留匹配）
- 下一个 n/N 会恢复搜索位置

### 4.4 复制选择和粘贴
- `useCopyOnSelect` - 拖选文本自动复制
- `useSelection` - 跟踪选择范围，支持拖动滚动边缘
- xterm.js 特定：支持 OSC 5 2 剪贴板协议

---

## 五、Message 组件 - 消息类型分派

### 5.1 支持的消息类型
1. **User Messages** - 用户输入（纯文本或带工具结果）
2. **Assistant Messages** - 模型响应（文本、思考、工具调用）
3. **System Messages** - 系统消息（警告、错误、提示）
4. **Attachment Messages** - 文件上传（图片、文档）
5. **Tool Result Messages** - 工具执行结果（bash 输出、file 读取等）
6. **Grouped Tool Use** - 多个工具调用折叠显示
7. **Compact Boundary** - 压缩提示（"以下 N 条消息已压缩"）
8. **Advisor Block** - 顾问建议（系统消息变体）

### 5.2 消息结构
```typescript
type NormalizedMessage = {
  uuid: string;
  role: 'user' | 'assistant' | 'system';
  content: (TextBlock | ToolUseBlock | ToolResultBlock | ThinkingBlock)[];
  tokens?: { input: number; output: number };
  // ... metadata
}
```

### 5.3 渲染选项
- `addMargin` - 消息间距
- `verbose` - 展开详细信息（思考块、完整工具调用等）
- `shouldAnimate` - 流式输入时动画
- `shouldShowDot` - 对齐点（对齐多行文本）
- `style: 'condensed'` - 紧凑模式
- `isTranscriptMode` - 导出/存档模式（禁用交互）

### 5.4 工具使用重新整理
- 多个连续 `tool_use` 块在一条消息内折叠显示
- `GroupedToolUseContent` 展示工具名称列表、折叠/展开控制
- 点击可展开单个工具的完整调用和结果

---

## 六、MessageRow - 单行消息布局

### 6.1 三列布局
```
┌─────────────────────────────────────────┐
│ [Role] [Content Start........................│
│        Content continues.....................│
│        ...............................End] │
└─────────────────────────────────────────┘

- [Role] - 用户/助手/系统标志（宽度固定）
- [Content] - 主消息体（弹性）
- [Metadata] - 令牌计数、时间戳等（可选）
```

### 6.2 多行消息处理
- Ink 的 `wrap: true` 自动按 width 折行
- 每行高度通过 Yoga 计算
- VirtualMessageList 缓存总高度

### 6.3 交互元素
- Hover 时显示折叠/复制/导出按钮
- 点击展开/折叠详细视图
- 文本选择支持（跨行）

---

## 七、当前 Sage 实现的差距

### 7.1 已有的部分（基础框架）
```rust
pub struct InteractiveMode {
    input_buffer: String,
    messages: Vec<ChatMessage>,
    running: bool,
    agent_rx: Option<mpsc::UnboundedReceiver<AgentDelta>>,
    is_thinking: bool,
    // ... 令牌统计
}

pub struct ChatMessage {
    pub role: MessageRole,  // User | Assistant | System
    pub content: String,
}
```

- ✅ 消息存储结构
- ✅ 基础 ratatui 渲染框架
- ✅ 事件循环和 agent 三角洲消费
- ✅ 终端初始化和模式管理

### 7.2 缺少的功能

#### 滚动系统
- ❌ Viewport 裁剪（只渲染可见行）
- ❌ Sticky scroll（自动跟随新消息）
- ❌ 命令式滚动 API
- ❌ 滚动位置缓存

#### 消息渲染
- ❌ 消息高度缓存（按宽度分层）
- ❌ 多行文本自动换行
- ❌ 工具调用的折叠/展开
- ❌ 消息类型特定的渲染（不同颜色、缩进等）

#### 键盘/滚轮处理
- ❌ j/k/g/G 快捷键
- ❌ Wheel 自适应加速（正常终端 vs xterm.js）
- ❌ 设备检测（鼠标 vs 轨迹板）
- ❌ 搜索上下文维护

#### 交互特性
- ❌ 消息展开/折叠（toggle verbose）
- ❌ 消息内搜索（/ 快捷键）
- ❌ 文本选择和复制
- ❌ 悬停时显示操作按钮

---

## 八、Rust 实现建议

### 8.1 架构分层
```
Layer 1: Message Model
  ├─ ChatMessage { role, content, ... }
  ├─ MessageType { User | Assistant | System | Tool | ... }
  └─ MessageMetadata { tokens, timestamp, ... }

Layer 2: Layout & Measurement
  ├─ Line wrapping (按 terminal width)
  ├─ Height cache { width -> total_height }
  └─ Viewport calculation { scroll_top -> visible_range }

Layer 3: Scroll Management
  ├─ ScrollState { top, pending_delta, is_sticky }
  ├─ ViewportRenderer { render(visible_messages) -> Frame }
  └─ InputHandler { j/k/g/G/Wheel -> ScrollDelta }

Layer 4: TUI Rendering
  ├─ ratatui::Terminal
  ├─ ratatui::Paragraph (with wrapping)
  └─ Frame composition
```

### 8.2 关键算法

#### 高度缓存
```rust
struct HeightCache {
    map: HashMap<(MessageId, TerminalWidth), LineCount>,
    // 清除策略：width 改变时全清
}
```

#### Viewport 计算
```rust
fn compute_visible_range(
    messages: &[ChatMessage],
    height_cache: &HeightCache,
    scroll_top: u16,
    viewport_height: u16,
) -> Range<usize> {
    // 二分查找第一条消息使其顶部 >= scroll_top
    // 二分查找最后一条消息使其底部 < scroll_top + viewport_height
}
```

#### 滚动增量处理
```rust
fn apply_scroll(
    state: &mut ScrollState,
    delta: ScrollDelta,  // Absolute | Relative | Bottom
    total_content_height: u16,
    viewport_height: u16,
) {
    match delta {
        Absolute(y) => {
            state.is_sticky = false;
            state.scroll_top = clamp(y, 0, total_height - viewport_height);
        },
        Relative(dy) => {
            state.pending_delta += dy;  // 累积
            state.is_sticky = false;
        },
        Bottom => {
            state.is_sticky = true;
            state.scroll_top = total_height - viewport_height;
        }
    }
}
```

### 8.3 渐进式实现策略
1. **阶段 1** - 基础滚动
   - 消息存储和 viewport 计算
   - 简单的 j/k 滚动
   - 无缓存（每次 render 都计算高度）

2. **阶段 2** - 滚动优化
   - 高度缓存
   - Sticky scroll
   - g/G 快捷键

3. **阶段 3** - 交互增强
   - 消息展开/折叠
   - 搜索支持
   - 文本选择

4. **阶段 4** - 高级输入
   - Wheel 自适应加速
   - 设备检测
   - Ctrl+U/D/B/F

---

## 九、性能考量

### 9.1 大消息历史（1000+ 条）
- CC 使用 viewport 裁剪：只 render O(20) 条可见消息
- 高度缓存避免重复测量
- 搜索索引缓存加速增量搜索

### 9.2 流式输出
- Sticky scroll 使最新消息始终可见
- 微任务批处理 wheel events
- 增量令牌计数（无需重新扫描整个历史）

### 9.3 Wheel 加速
- 避免每个 wheel event 都重新计算
- 使用指数衰减而非线性，快速滚动时无缓冲堆积

---

## 十、参考实现细节

### 10.1 ScrollBox 关键代码
- **位置锚点** - ScrollBox.tsx:130-139，`scrollToElement` 方法
- **粘性追踪** - ScrollBox.tsx:153-161，`scrollToBottom` 方法
- **微任务批处理** - ScrollBox.tsx:103-117，`scrollMutated` 函数

### 10.2 VirtualMessageList 关键代码
- **虚拟化计算** - VirtualMessageList.tsx:使用 `useVirtualScroll` hook
- **搜索缓存** - 默认提取器预处理文本，调用者传入缓存
- **高度测量** - `getFreshScrollHeight()` 直接读取 Yoga 而非缓存值

### 10.3 ScrollKeybindingHandler 关键代码
- **设备检测** - ScrollKeybindingHandler.tsx:54-82，反弹和空闲 gap 逻辑
- **加速曲线** - ScrollKeybindingHandler.tsx:84-100，衰减参数和稳态计算

---

## 总结

CC 的 TUI 系统通过分层架构、高度缓存、viewport 裁剪和复杂的输入加速算法，在数千条消息的历史中保持了 60fps 的流畅体验。Sage 的 Rust 实现应该：

1. **优先** - 消息存储、viewport 计算、基础滚动（影响 UX 最直接）
2. **次级** - 高度缓存、sticky scroll、搜索（用户高频操作）
3. **可选** - Wheel 加速细节、设备检测（高端优化）

重点是**正确的分层设计**，避免在 ratatui render 时进行高度测量——这会导致卡顿。
