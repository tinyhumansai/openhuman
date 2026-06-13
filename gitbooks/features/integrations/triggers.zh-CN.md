---
description: >-
  已连接集成（Gmail 新邮件、Notion 编辑、Stripe charge）的实时事件
  作为触发器到达，被分类器分类，并可自动触发智能体操作。
icon: bolt
---

# 触发器

已连接的集成不仅仅是智能体可以按需读取的地方。它也是**实时事件源**。当有人给你发邮件、编辑 Notion 页面、在你的某个仓库打开 GitHub Issue、在 Stripe 上给你的卡收费、或在 Slack 上给你发 DM 时，OpenHuman 几乎实时接收该事件，并可以决定是否要对其采取行动。

本页关于这条流水线：触发器如何到达、如何分类、以及触发器如何无需你输入一个字就变成完整的智能体操作。

## 什么是触发器

触发器是你所连接集成发布的外部事件。常见形态：

| 集成 | 示例触发器 |
| --- | --- |
| **Gmail** | `GMAIL_NEW_GMAIL_MESSAGE`，收件箱中的新邮件 |
| **Slack** | `SLACK_NEW_MESSAGE`，你被提及的频道/DM 消息 |
| **Notion** | `NOTION_PAGE_UPDATED`，被跟踪的页面有变化 |
| **GitHub** | `GITHUB_ISSUE_OPENED`、`GITHUB_PULL_REQUEST_OPENED`，你的仓库上 |
| **Stripe** | `STRIPE_CHARGE_SUCCEEDED`，你账户上的一笔成功 charge |
| **日历** | `GOOGLE_CALENDAR_EVENT_CREATED`，你日历上的新事件 |

完整集合来自为[第三方集成](README.zh-CN.md)提供支持的 [Composio](https://composio.dev) 连接器层。当连接活跃时，相关的触发器订阅会自动接入。

### Gmail OAuth 作用域

Gmail 触发器订阅需要所连接 Google 账户的邮件读取权限。新鲜的 OpenHuman Gmail 授权请求 `https://www.googleapis.com/auth/gmail.readonly`，这样 `GMAIL_NEW_GMAIL_MESSAGE` 可以启用，原生 Gmail 同步路径可以读取新邮件元数据。

如果旧 Gmail 连接在此作用域被请求之前创建，请从设置中重新连接 Gmail 然后再启用 Gmail 触发器。

## 触发器从哪里来，从头到尾

```text
┌────────────────────┐
│ third-party API │ Gmail / Slack / Notion / GitHub / ...
└─────────┬──────────┘
 │ webhook
 ▼
┌────────────────────┐
│ OpenHuman backend │ HMAC 验证 webhook，规范 payload
└─────────┬──────────┘
 │ Socket.IO 事件（"composio:trigger"）
 ▼
┌────────────────────┐
│ Rust core │ 在进程内事件总线上发布 DomainEvent::ComposioTriggerReceived
│（你的笔记本）│
└─────────┬──────────┘
 │
 ▼
┌────────────────────┐
│ Trigger Triage │ 分类：drop / acknowledge / react / escalate
└─────────┬──────────┘
 │
 ▼
┌────────────────────┐
│ 以下之一： │
│ - nothing │ ← drop
│ - memory note │ ← acknowledge
│ - Trigger Reactor │ ← react（1-2 个工具调用）
│ - Orchestrator │ ← escalate（完整多步规划）
└────────────────────┘
```

Webhook 永远不会被原始地到达你的机器。后端持有 OAuth token 并直接从第三方接收 webhook。它进行 HMAC 验证、规范 payload，并通过已认证的 socket 将其转发给你的 Rust core。你的笔记本在总线上看到一个干净的、经过验证的 `ComposioTriggerReceived` 事件，没有别的。

## 分类步骤

在任何操作运行之前，每个触发器都经过 [`trigger_triage`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/agent/agents/trigger_triage) 智能体。它的唯一工作是决定系统其余部分应该做什么。

它精确选择四种操作之一：

| 操作 | 发生什么 | 何时使用 |
| --- | --- | --- |
| **`drop`** | 什么也不做。触发器被静默记录并丢弃。 | 垃圾邮件、重复、不相关的噪音。默认用于你不在乎的东西。 |
| **`acknowledge`** | 持久化一条短期记忆笔记，不运行智能体。 | 值得记住的被动通知（"档案中创建了一个新页面"）。 |
| **`react`** | 使用一到两个工具调用运行 [`trigger_reactor`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/agent/agents/trigger_reactor) 智能体。 | 一个小的、单步的副作用：存储一条记忆条目、发布快速确认、将线程标记为已读。 |
| **`escalate`** | 全权交给带规划能力的 **orchestrator** 智能体。 | 需要推理、多步、或多技能的任何东西：起草回复、更新多个 Notion 页面、决定如何分类入站 issue。 |

分类智能体拥有与智能体其余部分相同的记忆和工作区上下文。它可以判断触发器是否与你现在正在做的事情相关、涉及哪些人、以及是否是你之前要求 OpenHuman 采取行动的那类事情。

## 触发器何时变成智能体操作

这就是区分"OpenHuman 有 Gmail 集成"和"OpenHuman 在值班你的收件箱"的部分：

- **`react`** 是廉价路径。Trigger Reactor 是一个有严格预算的窄专家，只有几个工具调用。它非常适合：写一条简短的记忆笔记说"看到 Stripe 新增一笔 $84 charge，客户 X，商户 Y"、静默将同一自动提醒标记为已处理因为你本周已经分类过两次、或存储用户以后可能想查找的事件的结构化记录。

- **`escalate`** 是重型路径。当分类智能体决定触发器需要真正的工作时，它将自包含的任务描述交给 Orchestrator。orchestrator 可以访问你完整的技能表面、工具、记忆和[潜意识循环](../subconscious.zh-CN.md)输出。从那里它可能：
  - 起草一封重要邮件的回复并排队等待你批准。
  - 为入站 issue 拉取相关的 Notion / Linear / Drive 上下文并写一条结构化评论。
  - 基于单个入站事件更新三个已连接系统（"这个客户的计划在 Stripe 变了，更新 HubSpot，在 #revenue 发帖，并在他们的 Notion 文件中添加一条笔记"）。
  - 判断触发器意味着一个会议刚刚被预定并为该通话预加载[会议智能体](../mascot/meeting-agents.zh-CN.md)。

两种情况下操作都在你的机器上运行，针对你的本地记忆树，使用与智能体其余部分相同的模型路由和工具表面。

## 为什么要一个分类步骤

跳过分类器并将每个触发器直接管道到 orchestrator 很有诱惑力。这是一个坏主意，有两个原因：

1. **大多数触发器是噪音。** 一个已连接的 Gmail 账户每小时触发数十个触发器，其中绝大多数是用户不在乎的。在每个上运行 orchestrator 会消耗预算并产生持续的后台活动流。
2. **不同的触发器值得不同的上限。** 一个自动 Stripe 收据和个人 Slack DM 不应该花相同的 token 数来处理。分类让廉价路径保持廉价，并将 orchestrator 保留给值得它的东西。

分类在快速模型层运行（参见[自动模型路由](../model-routing/README.zh-CN.md)），所以分类本身在亚秒级完成。

## 配置和退出

- **默认开启。** 一旦集成被连接，其触发器自动进入流水线。
- **退出。** 分类路径由 `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` 环境变量控制。设为 `1` / `true` / `yes` 关闭智能体分类并退回到仅被动日志记录。集成本身保持连接；只有自动操作行为被抑制。
- **每触发器设置。** 触发器设置（哪些集成和事件类型应该被评估）在**设置**下管理；底层 RPC 方法是 `update_composio_trigger_settings` / `get_composio_trigger_settings`。
- **审计日志。** 每个触发器，无论决策如何，都被写入触发器历史，这样你可以看到什么到达了、分类器决定了什么、以及（如果有的话）运行了什么。决策和升级也作为进程内总线上的 `TriggerEvaluated` / `TriggerEscalated` 事件发布，这意味着核心内部的任何东西都可以订阅它们。

## 隐私边界

触发器遵循与产品其余部分相同的边界（参见[隐私与安全](../privacy-and-security.zh-CN.md)）：

- 第三方 token 位于后端，永不在你的笔记本上。
- Webhook 在到达你的机器之前由后端进行 HMAC 验证。
- 触发器 payload 由你的本地 core 处理；分类和任何反应在你机器上运行，针对你的本地记忆树。
- `acknowledge` / `react` / `escalate` 路径写入的记忆笔记存储在你本地 SQLite 记忆树和 Markdown 存储库中，与任何其他来源相同。

## 开发者实现指针

- 分类智能体：`src/openhuman/agent/agents/trigger_triage/`
- Reactor 智能体：`src/openhuman/agent/agents/trigger_reactor/`
- Composio 总线订阅器：`src/openhuman/composio/bus.rs`（`ComposioTriggerSubscriber`）
- 触发器历史持久化：`src/openhuman/composio/trigger_history.rs`
- 领域事件：`DomainEvent::ComposioTriggerReceived`、`DomainEvent::TriggerEscalated` 在 `src/core/event_bus/events.rs` 中
- 触发器设置 RPC：`src/openhuman/config/` 中的 `update_composio_trigger_settings` / `get_composio_trigger_settings`

## 另见

* [第三方集成](README.zh-CN.md)，触发器来源的服务目录。
* [从集成自动拉取](../obsidian-wiki/auto-fetch.zh-CN.md)，轮询对应部分，定期将源数据摄入记忆树。
* [潜意识循环](../subconscious.zh-CN.md)，使用触发器上下文和记忆提前规划的背景循环。
* [会议智能体](../mascot/meeting-agents.zh-CN.md)，升级触发器可以落地的地方之一（日历事件有 Meet 链接）。