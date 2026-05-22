---
description: >-
  每隔二十分钟，OpenHuman 遍历每个活跃集成，将新数据整合进你的记忆树。
  无需提示词，无需编写轮询循环。
icon: arrows-rotate
---

# 自动拉取集成

大多数"AI 助手"是被动的：你提问，它们思考，它们回答。OpenHuman 则相反。它持续从你的技术栈中拉取数据，所以当你问"昨晚我的收件箱收到了什么？"时，答案已经在[记忆树](memory-tree.zh-CN.md)里了。

## 工作原理

一个单一的周期性调度器每二十分钟触发一次。每次触发时，它遍历每个活跃的[集成](../integrations/README.zh-CN.md)，查找匹配的原生 provider，如果该连接的距上次同步的时间足够长，就调用 `provider.sync(ctx, SyncReason::Periodic)`。

```text
每 20 分钟
    |
    v
遍历每个活跃连接（Gmail、Notion、GitHub……）
    |
    +--> 检查 sync_state（toolkit, connection_id）
    |       - 上次同步时间戳
    |       - 每日预算
    |       - 去重集合
    |       - 游标
    |
    +--> 如果间隔已过 -> provider.sync()
            |
            +--> 成功 -> record_sync_success(ts)
```

这里有几个关键点：

* **一个全局触发，而不是每个连接一个任务。** 每个用户的连接数很少；一个 20 分钟的触发周期足够了，而且 bookkeeping 很简单。
* **状态按 `(toolkit, connection_id)` 划分。** 每个连接有自己的游标、上次同步时间戳、去重集合和每日预算。重启时从中重建；即使重启后错过了一次周期性同步也无害，因为下一个触发周期会重新拾取。
* **原生同步与事件驱动路径共享。** 当 webhook 或 `on_connection_created` 事件触发非周期性同步时，它们在同一个 sync_state 上盖戳，所以调度器不会冗余地重新触发。
* **错误被记录并静默处理。** 调度器绝不能在其循环中 panic，否则周期性同步会在进程剩余生命周期内静默停止。

## 什么进入记忆树

每个 provider 负责定义自己的摄入逻辑。例如 Gmail provider 获取一页新消息，运行邮件规范化器，通过相同的手动 UI 摄入路径传输结果，块进入 SQLite，摘要 bucket 被填充，任何被触及的实体都会将主题树标记为脏。

其他 providers（GitHub、Slack、Notion……）遵循相同的形状：从游标后获取新项目 → 规范化 → 摄入到[记忆树](memory-tree.zh-CN.md)。

## 为什么是 20 分钟触发周期

最初设计每 60 秒运行一次。当连接了多个 provider 时，这意味着持续不断的 HTTP 获取和数据库写入，在笔记本上明显繁忙。二十分钟用一点延迟换取明显更少的前台负载。每个 provider 的 `sync_interval_secs` 仍然限制实际同步之间的**最小**延迟；全局触发周期只放宽上限。

## 调优和可见性

* **每个 provider 的间隔。** 每个原生 provider 声明自己的 `sync_interval_secs`，所以高流量工具包（Gmail）可以比低流量工具包（Stripe）更频繁地同步。
* **每日预算。** 每个连接有每日请求预算，以保持 API 成本和速率限制合理。
* **日志。** 同步活动以 debug 级别记录在 core 日志中。

## 另见

* [第三方集成](../integrations/README.zh-CN.md)。自动拉取运行的连接器层。
* [记忆树](memory-tree.zh-CN.md)。一切最终到达的地方。
* [智能 Token 压缩](../token-compression.zh-CN.md)。使"获取一切"保持低成本的原因。
