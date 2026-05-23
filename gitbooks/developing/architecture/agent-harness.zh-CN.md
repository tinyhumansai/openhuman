---
description: >-
  智能体轮次实际如何运行 —— 工具调用循环、子智能体分派、原型、分类、hook，以及围绕它们的成本/预算机制。
icon: layer-group
---

# Agent Harness

Agent Harness 是将用户消息（或 webhook 触发、cron tick）转变为完整的、使用工具的 LLM 交互的运行时。它拥有工具调用循环、子智能体分派、触发器-分类流水线和围绕它们的 hook 表面。它**不**拥有提供商 HTTP 传输、工具实现、提示部分组装或记忆存储 —— 那些是 harness 组合起来的独立领域。

本页先走过一个轮次中发生了什么，然后放大每个活动部件。

## 轮次的形态

每个轮次 —— 无论是用户刚输入消息、Telegram webhook 刚触发，还是 9am cron 刚 tick —— 都流经相同的生命周期：

```text
┌─ 入站 ─────────────────────────────────────────────────────────┐
│ 用户消息 · 渠道入站 · webhook · cron · composio 事件 │
└──────────────────────────┬────────────────────────────────────────┘
                           │
                           ▼  (仅外部触发器)
                ┌──────────────────────┐
                │   触发器分类         │  分类 → 丢弃 / 通知 /
                │   (小型本地 LLM)     │  生成 reactor / 生成 orchestrator
                └──────────┬───────────┘
                           │
                           ▼
            ┌──────────────────────────────┐
            │      Agent::turn()           │
            │  1. 恢复转录                 │
            │  2. 构建系统提示*            │
            │  3. 注入记忆上下文           │
            │  4. 进入工具调用循环 ────┼──► 提供商调用
            │  5. 分派工具调用  ────┼──► 工具执行 / 子智能体生成
            │  6. 上下文守卫 / 压缩        │
            │  7. 停止 hook 检查           │
            │  8. 最终助手文本             │
            └──────────┬───────────────────┘
                       │ 异步，在用户看到回复后
                       ▼
              ┌─────────────────┐
              │  轮次后         │  archivist · learning · 成本日志 ·
              │  hook           │  情景记忆索引
              └─────────────────┘

* 系统提示仅在第一轮构建 —— 后续轮次逐字复用渲染后的提示，
  以便推理后端的 KV-cache 前缀保持有效。
```

本页其余部分就是同一个图表，展开版。

## 会话和 `Agent::turn`

**会话**是 `Agent` 实例正在运行的实时对话。`Agent` 结构体拥有：

* 对话历史（系统 + 用户 + 助手 + 工具消息）。
* 要调用的提供商客户端（由[模型路由器](../../features/model-routing/)解析模型）。
* 模型可见的工具注册表。
* 在每条用户消息前为相关记忆补水的记忆加载器。
* 每轮预算 —— 最大工具迭代次数、最大 payload 大小、最大 USD 成本。

`Agent::turn(user_message)` 是热路径。在一个轮次中它：

1. **恢复会话转录**，如果这是一个新进程 —— 从磁盘重新加载精确的提供商消息，以便推理后端的 KV-cache 前缀仍然命中。
2. **构建系统提示**（仅在第一轮）。这拉入身份、soul、profile、记忆、已连接集成、可用工具、安全前言 —— 由提示部分构建器组装。
3. **注入记忆上下文**，通过记忆加载器为新用户消息注入：[记忆树](../../features/obsidian-wiki/memory-tree.zh-CN.md) 中的相关块，附带引用，使 UI 可以展示来源。
4. **进入工具调用循环**（下一节）。
5. **在后台生成轮次后 hook** —— 用户在 archivist / learning / 成本日志完成前就得到答案。

系统提示在后续轮次中**不**重建。即使是微小的字节变化也会使 KV-cache 前缀失效并强制完整重新 prefill，因此动态每轮上下文（记忆召回、新学习片段）作为用户可见的消息内容追加，而非拼接到系统提示中。

## 工具调用循环

在 `Agent::turn` 内部，工具调用循环是内部引擎。它最多运行 `max_tool_iterations` 轮（默认 10）：

```text
loop {
    1. 上下文守卫      - 如果历史太长，microcompact / autocompact
    2. 停止 hook 检查  - 预算上限、最大迭代次数、自定义 kill switch
    3. 提供商调用      - 发送消息 + 工具 spec，流式响应
    4. 解析响应        - 将助手文本与工具调用分离
    5. 如果没有工具调用 - 返回最终文本
    6. 执行工具调用    - 分派每个（下一节）
    7. 总结超大结果    - 将巨大工具输出路由到 summarizer 智能体
    8. 追加结果        - 将工具结果推入历史，再次循环
}
```

每次迭代都会发出实时 `AgentProgress` 事件，以便 UI 可以逐 token 渲染流式传输、"正在调用工具 X" 状态和每轮成本更新。

### 工具分派和工具调用方言

不同的 LLM 说不同的工具调用方言。harness 通过 `ToolDispatcher` trait 抽象了这一点，它有三个具体实现：

* **Native** —— 拥有一等工具调用 API 的提供商（Anthropic、OpenAI）。工具调用以结构化字段返回，不在文本体中。
* **XML** —— 未原生训练工具调用但可遵循指令的模型的 fallback。工具被包装在助手文本中的 `<tool_call>{...}</tool_call>` 标签内。
* **P-Format** —— 某些较小模型使用的紧凑文本格式。

dispatcher 按提供商选择，使循环本身方言无关。相同的循环代码驱动 Claude、GPT、Gemini 和本地 Ollama 模型。

### 循环中的上下文管理

长工具调用链可能超出上下文窗口。两层处理：

* **工具结果预算** —— 每个工具结果都对照每调用字节预算检查。任何超出的内容都会被硬截断，并附带解释性标记，以便模型知道它没有看到完整输出。
* **Microcompact / autocompact** —— 当总历史接近上下文窗口时，harness 在下次提供商调用前将旧轮次压缩为摘要。压缩后的历史保持系统提示和最近轮次不变（KV-cache 稳定性），并重写中间部分。

### 超大工具结果 —— summarizer 绕道

某些工具调用返回巨大的 payload —— Composio action dump 200 KB JSON、网页抓取返回 50 KB markdown、跨越数千行的日志上的 `file_read`。在 payload 中间硬截断会丢弃恰好落在截断点之后的任何内容。

当工具结果超过 summarizer 阈值时，它在进入父历史之前通过专用的 `summarizer` 子智能体路由。summarizer 按照保留标识符和关键事实的提取合约压缩 payload，父智能体只看到压缩后的摘要。当 summarization 失败或 payload 大到在其上支付 LLM 调用在经济上没有意义时，硬截断仍是下游的备用方案。

### 缺失命令的自愈

当代码执行器子智能体运行 shell 命令且运行时回答 "command not found" 时，自愈拦截器捕获错误，生成一个 `ToolMaker` 子智能体为缺失命令编写 polyfill 脚本，然后重试原始调用。每个命令有尝试上限，因此真正不可能的命令不会无限循环。

## 子智能体 —— orchestrator 模式

OpenHuman 是**多智能体**的。与用户聊天的智能体是 **Orchestrator** —— 一个高级别的、策略层面的智能体，决定何时直接回答、何时使用直接工具、何时生成专家子智能体。

### 为什么多智能体

一个知道一切的单个智能体也有一个小书大小的系统提示。将工作拆分到专家意味着：

* 每个子智能体获得一个**窄系统提示**，只有它需要的部分（可以剥离身份 / 记忆 / 安全前言）。
* 每个子智能体获得一个**过滤后的工具注册表** —— 集成智能体不需要文件系统工具，coder 不需要 Composio 目录。
* 子智能体历史永远不会泄露回父级 —— 父级看到一个紧凑的工具结果，而非内部对话。
* 更便宜的模型可以做叶子工作。Orchestrator 使用强推理模型；研究子智能体可能使用更快、更便宜的模型。

### 内置原型

每个原型位于 `agents/<name>/` 下，带一个 `agent.toml`（元数据、工具范围、模型提示）和一个提示：

| 原型 | Orchestrator 何时选择它 |
| ------------------- | --------------------------------------------------------------------------------------- |
| `orchestrator` | 顶层智能体。永远不会被另一个 orchestrator 生成。 |
| `planner` | 多步分解 —— 将复杂请求分解为有序子任务。 |
| `researcher` | 网页/文档查找、引用搜寻。 |
| `code_executor` | 在工作区中编写、运行和调试代码。 |
| `critic` | 代码审查、对另一个智能体输出的质量检查。 |
| `summarizer` | 压缩超大工具结果（由 harness 调用，通常不是模型调用）。 |
| `archivist` | 记忆蒸馏 —— 持久化什么、遗忘什么。 |
| `tool_maker` | 自愈 —— 为缺失的 shell 命令编写 polyfill。 |
| `tools_agent` | 任意工具绑定任务的通用专家。 |
| `integrations_agent`| 绑定到特定 Composio 工具包（Gmail、GitHub、Slack…）以执行该工具包的动作。|
| `trigger_triage` | 将传入的外部事件分类为丢弃 / 通知 / 生成 reactor / 生成智能体。 |
| `trigger_reactor` | 对分类后的触发器的轻量级反应，不需要完整的 orchestrator 轮次。 |
| `morning_briefing` | 由 cron 运行的精选每日摘要。 |
| `welcome` / `help` | Onboarding 流程。 |

自定义原型作为 TOML 文件发布在 `$OPENHUMAN_WORKSPACE/agents/*.toml`（或 `~/.openhuman/agents/*.toml` 用于用户全局专家）。自定义定义在 id 冲突时覆盖内置定义。

### 运行子智能体

当 orchestrator 调用 `spawn_subagent`（或 `delegate_*` 便捷工具之一）时，runner：

1. 从 task-local 读取父执行上下文 —— 父提供商、sandbox 模式、取消围栏、转录根。
2. 解析子智能体的模型 —— 继承父级、遵循提示（`fast` / `reasoning` / `summarization`），或固定到精确模型。
3. 按定义的 `tools`、`disallowed_tools` 和 `skill_filter` 过滤父级的工具注册表。在 `fork` 模式下，父级的完整注册表逐字继承。
4. 构建窄系统提示，省略定义要求剥离的部分。
5. 使用与父级相同的机制运行内部工具调用循环。
6. 返回一个紧凑的文本结果。子智能体内部历史永远不会拼接到父级中 —— orchestrator 看到一个单一的工具结果并继续。

对于不需要阻塞 orchestrator 轮次的任务，`spawn_worker_thread` 在后台运行子智能体，orchestrator 立即继续。

### 生成层级和 tiers

并非每个智能体都被允许生成每个其他智能体。harness 建模了一个三层层级，镜像模型之间的成本 / 延迟 / 思考深度拆分：

```text
Chat        (快速，UX 聚焦 —— 例如 orchestrator 使用 `chat` 提示)
  │
  ├─► Worker      ◄─── 快速路径：一次委托，叶子做工作
  │
  └─► Reasoning   (慢速，深度思考 —— 例如 planner 使用 `reasoning` 提示)
        │
        └─► Worker  ◄─── 深度路径：reasoning 分解，workers 执行
```

每个 `AgentDefinition` 携带一个 `agent_tier` 字段（`chat` / `reasoning` / `worker`，默认 `worker`）。契约：

| Tier | 可以生成 | 禁止生成 | 典型成员 |
| ------------ | ----------------- | ---------------------------- | -------------------------------------------------------- |
| `chat` | `reasoning`, `worker` | 另一个 `chat` | `orchestrator` |
| `reasoning` | `worker` | 另一个 `reasoning`、任何 `chat` | `planner`（当今的规范代表） |
| `worker` | nothing[^1] | 任何东西 | researcher、code_executor、critic、archivist、tool_maker、integrations_agent、… |

[^1]: Skill-wildcard 条目（`{ skills = "*" }`）被豁免，因为它们坍缩为单个 `delegate_to_integrations_agent` 工具，其目标是 worker —— 它们是扇出委托表面，不是递归生成。

**为什么有这些规则。**
- *Chat → chat 毫无意义。* Chat tier 存在是为了 snappy UX。Chat 智能体生成另一个 chat 智能体只是加倍 TTFT 并燃烧 token 而不购买任何新能力。
- *Reasoning → reasoning 会爆炸深度。* Reasoning tier 很昂贵。Reasoning 智能体链倾向于重新分解相同问题并创建失控的层级。
- *Worker → anything 混合执行和编排。* Workers 是叶子，因此父级总是看到一个紧凑结果，而非嵌套委托的转录。

**强制执行。** 两层：

1. **加载时（静态）。** [`agents::loader::validate_tier_hierarchy`](../../../src/openhuman/agent/agents/loader.rs) 在合并的注册表（内置 + workspace TOML）上运行，并拒绝启动列出同级或 worker-with-subagents 条目的注册表。内置原型在编译测试时检查；用户发布的 TOML 在 workspace 加载时检查。
2. **运行时深度门禁（动态）。** 独立于 tier，子智能体 runner 通过 task-local 计数器将总生成链深度限制为 `MAX_SPAWN_DEPTH = 3`，该计数器在 `run_subagent` 之间递增，作为 `SpawnDepthExceeded` 智能体错误展示。这使得一个删除了 tier 注释的用户发布 TOML 仍然无法递归超过三跳。

> **状态：** 加载时 tier 检查、`agent_tier` 字段和运行时深度计数器 task-local 已上线。深度由静态加载器契约和运行时 `MAX_SPAWN_DEPTH = 3` 守卫共同限制。

### 工具包特定专家

对于具有数百个动作的 Composio 工具包（仅 GitHub 就有 500+），将每个动作加载到子智能体的工具集中会膨胀提示大小。harness 通过廉价的纯 CPU 过滤器（动词检测、token 重叠、动词对齐提升）将工具包的动作与父级精炼的任务提示进行排名，并仅将排名靠前的子集加载到子智能体中。无需模型调用，纯启发式 —— 快速且可解释。

## 分类 —— 处理外部触发器

当 webhook 触发、cron tick 或 Composio 事件到达时，系统不能直接将它们交给 orchestrator。大多数触发器是噪音；有些值得通知；只有少数值得完整的智能体轮次。**触发器-分类流水线**是门禁。

```text
TriggerEnvelope ──► run_triage ──► TriageDecision ──► apply_decision
                       │                                     │
                       │                                     ├─► 丢弃 (噪音)
                       │                                     ├─► 仅通知
                       │                                     ├─► 生成 trigger_reactor
                       │                                     └─► 生成 orchestrator
                       │
                       └── 小型本地 LLM（云端 LLM 重试 fallback）
```

evaluator 有意保持廉价 —— 在可用时使用小型本地模型，重试时 fallback 到远程模型。决策被缓存，因此相同的触发器不会重新分类。只有升级到"生成 orchestrator"的触发器才会通过完整的 `Agent::turn` 机制。

## Hook —— 可观测性和策略杠杆

两个 hook 表面包裹循环，位于两端：

### 停止 hook（轮次中）

停止 hook 在工具调用循环的**迭代之间**触发。它们是预算上限、速率限制和自定义 kill switch 的策略杠杆。内置 hook：

* **预算停止 hook** —— 使用每轮成本累加器限制轮次的累计 USD 成本。
* **最大迭代次数停止 hook** —— 从智能体持久配置外部限制迭代次数。

返回 `Stop` 的 hook 会以清晰的原因中止循环，调用者可以将该原因展示给用户。停止 hook 与中断（下一节）不同：它们是策略驱动的，不是用户驱动的。

### 轮次后 hook

轮次后 hook 在轮次**完成后**触发，在后台。它们获得 `TurnContext` 快照 —— 用户消息、助手响应、每个工具调用及其参数和结果、总 wall-clock、迭代次数、会话 ID。内置消费者：

* **Archivist** —— 蒸馏轮次中哪些事实值得持久化到长期记忆。
* **Learning** —— 为 reflection、工具跟踪器和用户 profile 更新提供输入。
* **成本日志** —— 最终每轮成本行。
* **情景记忆索引** —— 将轮次作为块写入[记忆树](../../features/obsidian-wiki/memory-tree.zh-CN.md)以供未来召回。

Hook 通过 `tokio::spawn` 运行，因此用户在它们完成前就得到了答案。

## 中断 —— 优雅取消

`InterruptFence` 在循环的固定安全点检查 —— 每次工具执行前、每次子智能体生成前、每次提供商调用前。当用户按下 Ctrl+C 或发送 `/stop`：

* 围栏翻转。
* 每个正在运行的子智能体看到相同的 flag（通过 `Arc` 共享）并在其下一个检查点退出。
* 进行中的提供商流被丢弃。
* Archivist 仍然使用任何存在的部分上下文触发，因此对话不会丢失。

中断是用户驱动的；停止 hook 是策略驱动的。它们共享底层的"干净停止循环"管道，但从不同侧面进入。

## 成本核算

每个提供商响应携带一个 `UsageInfo` 块 —— 输入 token、输出 token、缓存输入 token，以及由 OpenHuman 后端填充的权威 `charged_amount_usd`。`TurnCost` 在一个轮次内对每个提供商调用求和，以便 harness 可以：

* 通过进度通道发出每轮成本遥测。
* 为预算停止 hook 提供输入，使失控的轮次在循环中自我切断。
* 记录精确的轮次结束成本行。

当后端不展示收费金额时（旧构建、不通过它计费的提供商），一个小的每 tier 费率表提供 token 费率 floor 估计。后端直接成本在可用时总是优先。

## Fork 上下文 —— 跨 harness 的 KV-cache 复用

harness 使用 task-local `ParentExecutionContext` 将父状态线程化到子智能体中，而不会爆炸每个函数签名。相同的模式携带当前 sandbox 模式、中断围栏和停止 hook 列表。继承父级提供商、模型和提示前缀的子智能体可以在推理后端上**共享父级的 KV-cache 前缀** —— 比从头重新 prefill 明显更便宜。

## 自愈回顾

几个小型自适应系统位于主循环之上：

* **缺失命令的自愈** —— `ToolMaker` polyfill，有上限的重试尝试。
* **Payload summarizer 断路器** —— 会话中连续三次子智能体失败会禁用 summarization，fallback 到截断。
* **分类本地-vs-远程重试** —— 本地 LLM 优先；解析失败时远程 fallback。

这些都不会改变循环的形状 —— 它们只是让常见故障模式无需用户干预即可恢复。

## 代码中该看哪里

harness 完全位于 `src/openhuman/agent/` 下。该目录中的 README 枚举了公共表面；负载最重的文件是：

| 文件 / 目录 | 里面有什么 |
| ----------------------------- | ----------------------------------------------------------------- |
| `harness/session/turn.rs` | `Agent::turn` —— 上述生命周期。 |
| `harness/tool_loop.rs` | 内部工具调用循环。 |
| `harness/subagent_runner/` | `run_subagent`、fork 模式、超大结果交接。 |
| `harness/definition.rs` | `AgentDefinition` —— 原型声明的内容。 |
| `harness/tool_filter.rs` | 集成子智能体的工具包动作排名。 |
| `harness/payload_summarizer.rs` | 超大工具结果绕道。 |
| `harness/self_healing.rs` | 缺失命令拦截器。 |
| `harness/interrupt.rs` | 取消围栏。 |
| `dispatcher.rs` | 工具调用方言抽象。 |
| `triage/` | 外部触发器分类 + 升级。 |
| `agents/` | 内置原型 —— 每个智能体一个子目录。 |
| `hooks.rs` / `stop_hooks.rs` | 轮次后和轮次中 hook 表面。 |
| `cost.rs` | 每轮 USD/token 核算。 |
| `progress.rs` | 到 UI 的实时进度事件。 |
| `memory_loader.rs` | 每条用户消息的记忆树上下文注入。 |

## 另请参阅

* [架构概览](README.zh-CN.md) —— harness 在更大图景中的位置。
* [记忆树](../../features/obsidian-wiki/memory-tree.zh-CN.md) —— 记忆加载器从中读取、轮次后 hook 写入的内容。
* [自动模型路由](../../features/model-routing/README.zh-CN.md) —— `model: "hint:reasoning"` 如何解析为具体的提供商+模型。
* [原生工具 —— 智能体协调](../../features/native-tools/agent-coordination.zh-CN.md) —— `spawn_subagent`、`delegate_*`、`todo_write` 的用户可见表面。
