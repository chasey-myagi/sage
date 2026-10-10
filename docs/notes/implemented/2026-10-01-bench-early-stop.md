# Benchmark 记录编辑后直接结束的比例

## Problem

任务状态不能区分 agent 编辑后直接结束与尝试运行检查后结束，需要从既有 Log 提取可比较的行为信号。

## Decision

增加 `stoppedEarly` 和 `stoppedEarlyRate`：成功 edit/write 后没有 bash 结果、以 done 结束且 diff 非空时计入。工具名由 tools 包统一定义。统计来自既有 Log 和评分前 diff，不增加模型输入。

## Alternatives considered

解析 bash 命令判断是否跑测试容易漏掉脚本和组合命令，因此只记录工具顺序。把该信号作为任务失败依据会混淆行为与测试结果，因此保留原任务状态。

## Consequences

任何 bash 结果（包括失败）都会清除信号；bash 内编辑不会被识别。该指标不能证明验证质量。会话或 diff 不可用、子进程超时未读取 diff 时为 null；比例分母仍包含未知任务。结果与报告新增字段，原有精确对象断言增加对应字段，其他断言不变。

## Testing

通过 runCli 入口读取构造的 JSONL Log，覆盖编辑顺序、失败编辑、失败 bash、max_turns、空 diff、缺会话、diff 失败与子进程超时。修复前两种未知 diff 情形误报 true，新增断言要求 null，先红后绿。报告测试覆盖跨运行比例与差值。全部测试使用本地替身，不请求模型。
