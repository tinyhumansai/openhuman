---
description: >-
  可选的 `Memory` trait 后端，委托给本地运行的 agentmemory REST 服务器，
  适用于在 Claude Code、Cursor、Codex、OpenCode 和 OpenHuman 间
  自托管 agentmemory 的用户。
icon: database
---

# agentmemory 后端

OpenHuman 默认的 `Memory` trait 后端是 `sqlite`——即 [记忆树](memory-tree.zh-CN.md) 中记录的统一存储。对于已经在本地运行 [agentmemory](https://github.com/rohitg00/agentmemory) 的用户——通常是因为他们希望在 Claude Code、Cursor、Codex、OpenCode 和 OpenHuman 之间共享单一持久化存储——OpenHuman 暴露了一个可选后端，将每个 trait 调用代理到 agentmemory 的 REST 层面。

选择 `backend = "agentmemory"` 会跳过 OpenHuman 的 SQLite + 嵌入器路径。agentmemory 拥有存储、嵌入和检索层。OpenHuman 成为一个精简的 REST 客户端。

## 何时使用

在以下情况下使用 agentmemory 后端：

- 你已经为一个或多个编码智能体运行 `npx -y @agentmemory/agentmemory`，并希望 OpenHuman 共享相同的持久化存储。
- 你希望混合 BM25 + 向量 + 图检索，而无需在 OpenHuman 端配置单独的嵌入器。
- 你偏好 agentmemory 的生命周期（整合、保留评分、自动遗忘、图提取）而不是 OpenHuman 的统一存储。

在以下情况下保持默认的 `sqlite` 后端：

- 你想要完全自包含的单进程操作，无外部守护进程依赖。
- 你依赖 OpenHuman 特定的记忆树功能（分块、密封、摘要树），这些功能在 SQLite 存储之上运行。记忆树流水线不受 trait 后端影响——它在主机的文档存储上操作，正交——但 agentmemory 后端在你已经在其他智能体上标准化使用 agentmemory 时最有价值。

## 快速开始

1. **安装 + 启动 agentmemory**（一个终端）：

   ```bash
   npx -y @agentmemory/agentmemory
   ```

   默认为 `http://localhost:3111`（REST）+ `ws://localhost:49134`（引擎）。首次启动在 `~/.agentmemory/.hmac` 生成 HMAC 密钥并打印一次。

2. **在 `config.toml` 中将 OpenHuman 指向它**：

   ```toml
   [memory]
   backend = "agentmemory"
   # 以下为默认值——仅在覆盖时设置。
   # agentmemory_url        = "http://localhost:3111"
   # agentmemory_secret     = ""           # HMAC bearer token，可选
   # agentmemory_timeout_ms = 5000
   ```

3. **重启 OpenHuman**。Factory 会跳过 SQLite 路径并记录 `[memory::factory] using agentmemory backend at <url>`。

就这样。现有的 OpenHuman 调用点（`store`、`recall`、`get`、`list`、`forget`、`namespace_summaries`、`count`、`health_check`）保持不变。

## 配置 keys

| 字段 | 默认值 | 用途 |
| --- | --- | --- |
| `agentmemory_url` | `http://localhost:3111` | agentmemory REST 服务器的基础 URL |
| `agentmemory_secret` | 无 | 可选的 HMAC bearer token。作为 `Authorization: Bearer <secret>` 发送 |
| `agentmemory_timeout_ms` | `5000` | 每个请求的 reqwest 超时 |

当 `backend == "agentmemory"` 时，以下现有 `MemoryConfig` 字段被**忽略**——agentmemory 通过 `~/.agentmemory/.env` 管理自己的嵌入堆栈：

- `embedding_provider`
- `embedding_model`
- `embedding_dimensions`
- `sqlite_open_timeout_secs`

在此路径上设置它们是空操作。本地 AI Ollama 健康检查也不在此路径上运行——agentmemory 的守护进程管理自己的嵌入器生命周期。

## 字段映射

OpenHuman 的 `MemoryEntry` ↔ agentmemory 传输行：

| OpenHuman 字段 | agentmemory 字段 | 备注 |
| --- | --- | --- |
| `namespace` | `project` | 空时默认为 `"default"` |
| `key` | `title` | |
| `content` | `content` | |
| `id` | `id` | agentmemory 生成的（`mem_<rand>`） |
| `category: Core` | `type: "fact"` | |
| `category: Daily` | `type: "conversation"` | |
| `category: Conversation` | `type: "conversation"` | |
| `category: Custom(s)` | `type: "fact"` + `concepts: [s]` | 自定义标签滚入 concepts 数组以保持可查询性 |
| `session_id` | `sessionIds: [...]` | OpenHuman 暴露单个 id；agentmemory 持久化一个数组 |
| `timestamp` | `updatedAt`（RFC3339） | 如果 `updatedAt` 缺失则回退到 `createdAt` |
| `score`（仅召回命中） | smart-search `score` | 在 `recall` 响应中填充，`get` / `list` 时为 `None` |

agentmemory 携带额外字段——`concepts`（自动提取）、`files`（路径标签）、`strength`（保留评分）、`version`、`supersedes`（生命周期链）——此后端保留为默认值。它们是 agentmemory 生命周期层的内部字段，不需要通过 OpenHuman 的 trait 进行往返。

## Trait 方法 → 端点

| `Memory` 方法 | agentmemory REST | 备注 |
| --- | --- | --- |
| `store` | `POST /agentmemory/remember` | `{project, title, content, type, concepts, sessionIds}` |
| `recall` | `POST /agentmemory/smart-search` | 混合 BM25 + 向量 + 图 |
| `get` | `POST /agentmemory/smart-search` | + 客户端精确 title 过滤 |
| `list` | `GET /agentmemory/memories?latest=true&project=<ns>` | |
| `forget` | `get(ns, key)` → `POST /agentmemory/forget` | 两步：先解析 id 再 forget |
| `namespace_summaries` | `GET /agentmemory/projects` | 返回 `[{name, count, lastUpdated}]` |
| `count` | `GET /agentmemory/health` | 读取 `memories` 字段 |
| `health_check` | `GET /agentmemory/livez` | |

`RecallOpts.category`、`RecallOpts.session_id` 和 `RecallOpts.min_score` 作为**客户端过滤**应用于 smart-search 响应。agentmemory 的 REST 面今天不将它们作为服务器端过滤器暴露。对于非常大的召回窗口（limit > 100），建议发出更严格的查询字符串以减少服务器端工作，而不是依赖客户端后过滤。

## 安全性

当 `agentmemory_secret` 被设置时，客户端遵守 agentmemory 的 v0.9.12 明文 Bearer 守卫约定：

- **环回主机**（`localhost`、`127.0.0.1`、`::1`）上的 `http://` —— 允许。本地开发路径。
- **`https://`** 到任何主机 —— 允许。
- **到非环回主机的明文 HTTP** —— 在构造时发出一次性 stderr 警告。Bearer 在线路上是可观察的。
- **`AGENTMEMORY_REQUIRE_HTTPS=1`**（进程环境，ASCII 大小写不敏感，匹配 `1` 或 `true`）—— 将警告升级为构造时的硬性拒绝。后端启动失败而不是静默泄露 bearer。

生产部署应设置 `AGENTMEMORY_REQUIRE_HTTPS=1`，这样配置错误的 TLS 终结器会明显报错，而不是静默泄露。

明文 bearer guard 镜像了 agentmemory [PR #315](https://github.com/rohitg00/agentmemory/pull/315) 中的集成插件 guard，因此在 Hermes / OpenClaw / pi 上看到过相同警告的操作员会在 OpenHuman 上认出它。

## 故障模式

| 故障 | 后端行为 |
| --- | --- |
| 启动时守护进程不可达 | `from_config` 成功（URL 解析），但首次调用时 `health_check()` 返回 false。Trait 方法向上冒泡 `reqwest` 传输错误 |
| 网络超时 | 按 trait 约定返回 `anyhow::Error`；浮出到调用者 |
| 4xx / 5xx 响应 | 带状态 + body 片段的 `anyhow::Error` |
| Bearer 通过明文非环回（无环境变量） | 一次性 stderr 警告，请求继续 |
| Bearer 通过明文非环回 + `AGENTMEMORY_REQUIRE_HTTPS=1` | 构造时硬性拒绝 |
| 空的 `agentmemory_url` | 构造时硬性拒绝并提示留空以使用默认值 |
| 无效的 URL 语法 | 构造时硬性拒绝并附带解析器错误 |

**不会自动回退到 SQLite。** 如果守护进程在启动时未运行，后端会明显抛出传输错误。操作员在 `config.toml` 中切回 `backend = "sqlite"` 以恢复。理由：静默的 SQLite 回退会隐藏配置错误的守护进程——"私密、简单、可预测"胜过"神奇容忍"。

## 性能说明

后端是一个精简的 REST 代理——每个 trait 调用增加一个 HTTP 往返。实际影响：

- `store` 和 `forget` 是单 RTT。
- `recall`、`get`、`list` 是单 RTT。
- 对未知 key 的 `forget` 是两个 RTT（隐式 `get` 查找 + 一个空操作确认）。调用者可以通过检查先前 `list` 的返回值来短路这个。
- agentmemory 的 REST 默认是 `127.0.0.1` —— 同主机延迟低于一毫秒。通过 HTTPS 终结的管理部署，预期每个 RTT 约 10–30ms。
- 默认每请求超时为 5 秒。如果在 iii 引擎冷启动时看到间歇性超时，增加 `agentmemory_timeout_ms`；agentmemory 长时间空闲后的第一次请求延迟可达 3–5 秒，取决于持久化状态。

## 迁移：从 SQLite 到 agentmemory

目前没有原地迁移。建议路径：

1. 通过 OpenHuman 现有的导出 RPC（或直接 SQL）从 SQLite 存储导出你现有的记忆。
2. 遍历导出，将每一行 POST 到 `/agentmemory/remember`，使用相同的 `project` + `title` + `content`。agentmemory 将分配新 id；OpenHuman 端在首次 `list` 时获取它们。
3. 设置 `backend = "agentmemory"` 并重启。

专门的批量导入路径作为后续跟进。

## 实现参考

仓库内文件：

- [`store/agentmemory/mod.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/mod.rs) —— 模块表面
- [`store/agentmemory/backend.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/backend.rs) —— `impl Memory for AgentMemoryBackend`
- [`store/agentmemory/client.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/client.rs) —— reqwest 包装器 + 明文 bearer guard
- [`store/agentmemory/mapping.rs`](https://github.com/tinyhumansai/openhuman/tree/main/src/openhuman/memory/store/agentmemory/mapping.rs) —— `MemoryEntry` ↔ agentmemory JSON
- [`tests/agentmemory_backend.rs`](https://github.com/tinyhumansai/openhuman/tree/main/tests/agentmemory_backend.rs) —— 12 个 axum-mock 集成测试

相关的上游：

- agentmemory 仓库 —— <https://github.com/rohitg00/agentmemory>
- agentmemory REST 约定 —— `~/.agentmemory/.env` keys + 端点列表在 agentmemory README 中
- v0.9.12 明文 bearer guard —— agentmemory PR #315
