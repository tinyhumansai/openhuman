---
description: 持久化的、工具作用域规则，用于安全关键型指引和学习成果。
icon: shield-check
lang: zh-CN
---

# 工具级记忆

工具级记忆层捕获关于智能体应如何使用特定工具的**可执行指引**——它与[记忆工具](memory-tools.zh-CN.md)的通用召回不同，也与 `tool_effectiveness` 统计命名空间相区别。它是把"永远不要给 Sarah 发邮件"转化为智能体在每一轮后续中都必须遵守的硬约束的表面。

它实现了 [issue #1400](https://github.com/tinyhumansai/openhuman/issues/1400)——一个用于持久化学习成果和高优先级规则的一流存储与检索系统。

## 存储内容

每个工具都有自己的命名空间 **`tool-{tool_name}`**，与 `global`、`skill-{id}` 以及仅用于统计的 `tool_effectiveness` 命名空间相区分。在其内部，每条记录都是一个 `ToolMemoryRule`：

| 字段 | 用途 |
| ---- | ---- |
| `id` | 每条规则的稳定 UUID。Upsert 会复用相同 id。 |
| `tool_name` | 规则适用的工具（例如 `send_email`、`shell`）。 |
| `rule` | 智能体必须遵循的自然语言指引。 |
| `priority` | `critical`、`high` 或 `normal`。驱动检索 + 压缩策略。 |
| `source` | `user_explicit`、`post_turn` 或 `programmatic`——来源。 |
| `tags` | 自由标签（`safety`、`permission`……）。 |
| `created_at` / `updated_at` | RFC3339 时间戳。 |

统计（`tool_effectiveness/tool/{name}`）和规则（`tool-{name}/rule/{id}`）按设计位于*不同*的命名空间——一个追踪"发生了什么"，另一个追踪"对此该做什么"。

## 优先级层级

| 优先级 | 存储位置 | 抗压缩？ |
| ------ | -------- | -------- |
| `critical` | 通过 `ToolMemoryRulesSection` 钉入**系统提示**。 | **是**——系统提示按 session 冻结，不会被 mid-session 压缩器重写。 |
| `high` | 同一块系统提示中，排在 critical 之后。 | **是**——机制相同。 |
| `normal` | 存储在命名空间中；通过 `memory_recall` 按需检索。 | 否——与任何其他命名空间记忆一样可被压缩。 |

抗压缩属性是结构性的：critical 和 high 规则 riding 在*系统提示*中，而推理后端的 prefix cache 会在整个 session 期间保持其冻结。没有任何方式能让 token 压缩静默丢弃一条 `critical` 规则。

## 捕获流水线

每轮之后有两条自动捕获路径触发（通过 `ToolMemoryCaptureHook`）：

1. **用户指令**——用户消息中的 `never <verb> <noun>`、`don't <verb> ...`、`do not <verb> ...` 或 `stop <verb>ing ...` 等句子会被提升为匹配工具的 **Critical** 规则。通用名词别名将 `"email"` 映射到名为 `send_email` 的工具，`"shell"` 映射到 `bash`/`exec` 等；当没有别名匹配时，规则会落在该轮次中第一个运行的工具上，使其保持在相关调用现场附近。
2. **重复工具失败**——在一轮中失败两次或以上的工具会获得一条 **Normal** 优先级的观察记录，失败类别被内联摘要，以便智能体下次考虑该工具时有上下文。

当学习子系统开启时，该 hook 默认启用。用 `OPENHUMAN_LEARNING_TOOL_MEMORY_CAPTURE_ENABLED=0` 选择性禁用。

## 工具选择时的检索

在 session 开始时，harness 通过 `ToolMemoryStore::rules_for_prompt` 预取每条 Critical 和 High 规则，将它们渲染到 `## Tool-scoped rules` 块中，并把该块钉入系统提示。因为提示在 session 生命周期内被冻结，这些规则在每一轮的工具选择时——以及任何实际工具执行之前——都是可见的。

低优先级指引不占用提示预算；智能体通过针对 `tool-{name}` 命名空间调用 `memory_recall` 按需获取它们。

## RPC 表面

`memory` 命名空间下暴露六个方法：

| 方法 | 用途 |
| ---- | ---- |
| `memory.tool_rule_put` | Upsert 一条规则。对安全关键型条目使用 `priority='critical'`。 |
| `memory.tool_rule_get` | 通过 `(tool_name, id)` 获取一条规则。 |
| `memory.tool_rule_list` | 列出某工具的所有规则，按优先级 + 新鲜度排序。 |
| `memory.tool_rule_delete` | 删除一条规则。 |
| `memory.tool_rules_for_prompt` | 返回渲染后的 Markdown 块 + 结构化快照——session builder 所钉入的内容。 |
| `memory.tool_rules_json` | 原始 JSON 列表（供信封消费者使用）。 |

JSON payload 使用 snake_case（`priority: "critical"`、`source: "user_explicit"`）。每个方法都经过与其他记忆 RPC 相同的 `active_memory_client` 管道。

## 端到端安全场景

"永远不要给 Sarah 发邮件"路径已被回归测试覆盖：

1. 用户在调用了 `send_email` 的轮次中说 *"Never email Sarah at sarah@example.com."*
2. `ToolMemoryCaptureHook` 提取该指令，将 `email` 别名映射到 `send_email` 工具，并在 `tool-send_email/rule/{uuid}` 下写入一条 Critical 规则。
3. 在下一个 session 中，`prefetch_tool_memory_rules_blocking` 拉取每条 Critical 和 High 规则，session builder 将 `ToolMemoryRulesSection` 追加到系统提示。
4. 智能体在选择工具之前就看到 `### \`send_email\`` 后跟 `- **[critical]** Never email Sarah at sarah@example.com.`，并且该规则在任何 mid-session token 压缩中都能存活。

覆盖率与集成测试位于 `src/openhuman/memory/tool_memory/`。

## 另请参阅

- [记忆工具](memory-tools.zh-CN.md)——通用 `recall`、`store`、`forget`。
- [智能 Token 压缩](../token-compression.zh-CN.md)——系统提示被保护免受的内容。
