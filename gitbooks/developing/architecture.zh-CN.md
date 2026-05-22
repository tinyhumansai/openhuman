---
description: OpenHuman 代码库的深度架构参考 —— 仓库布局、运行时范围、双 socket 同步、RPC 流程。
icon: code-branch
lang: zh-CN
---

# OpenHuman 架构

**基于 Rust 构建的加密社区 AI 超级助手。**

OpenHuman 是一款为加密货币生态系统量身打造的跨平台通信与自动化平台。单一的 React + Rust（Tauri）代码库可以面向多个平台；**我们目前为用户文档和发布的仅是桌面端** —— **Windows、macOS 和 Linux**。Android、iOS 和 Web **尚未**在当前文档或发布中支持。技术栈包括一个托管的 Node.js 运行时，用于支持工具能力的技能；持久化的 Rust 原生 WebSocket 基础设施；以及一个 AI 工具协议，让语言模型实时调用任何已连接的服务。

---

## 仓库布局（monorepo）

| 路径 | 内容 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **`app/`** | Yarn workspace **`openhuman-app`**：Vite/React UI（`app/src/`）、Tauri 壳层（`app/src-tauri/`）、Vitest 测试 |
| **仓库根目录 `src/`** | Rust **`openhuman_core`** 库 + **`openhuman-core`** CLI 二进制文件 —— 核心服务器、JSON-RPC、一等 JavaScript 运行时（`src/openhuman/javascript/`），由托管的 Node.js 实现驱动、频道、内存等 |
| **`Cargo.toml`**（根目录） | 构建 `openhuman-core` 二进制文件（`cargo build --bin openhuman-core`），staging 到 `app/src-tauri/binaries/` 以供桌面打包 |
| **`skills/`** | 运行时消耗的技能包 |
| **`docs/`** | 本书 + 每棵树指南（`docs/src/`、`docs/src-tauri/`） |

桌面应用 **WebView** 从 `app/` 加载 UI；繁重的 RPC 和技能在 **`openhuman-core`** 进程中运行，可通过 HTTP 从 Tauri 主机访问（`core_rpc_relay`）。

---

## 平台覆盖范围

**今天支持的（终端用户）：** 桌面端。Windows、macOS、Linux（原生安装包）。

**尚未支持：** Android、iOS、独立 Web 客户端（仓库中可能以实验性目标存在；不要视为产品就绪）。

```text
                        OpenHuman（已发布）
                            |
                         Desktop
                    /      |      \
               Windows   macOS   Linux
                x64      x64     x64
               ARM64    ARM64   ARM64
```

Tauri v2 将 Rust 核心编译为每个平台的原生二进制文件，将 React 前端作为轻量级 WebView 嵌入。桌面构建产出 `.dmg`、`.msi`、`.AppImage` 和 `.deb` 安装包。额外目标（移动端、Web）在明确文档化支持之前均超出范围。

---

## 高层架构

```text
+------------------------------------------------------------------+
|                        React 前端                                |
|  Redux Toolkit  |  Socket.io 客户端  |  MCP 传输层  |  UI      |
+------------------------------------------------------------------+
                          |  Tauri IPC 桥接  |
+------------------------------------------------------------------+
|                        Rust 核心引擎                             |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  QuickJS 技能    |  |  Socket 管理器   |  |  AI 加密        | |
|  |  运行时引擎      |  |  (持久化 WS)     |  |  & 内存存储     | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  技能注册表      |  |  Cron 调度器     |  |  会话 & 认证    | |
|  |  & 桥接 API      |  |  (5s tick 循环)  |  |  管理           | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |   Telegram       |  |  SQLite 存储     |  |  OS 钥匙串      | |
|  |   集成           |  |  (rusqlite)      |  |  集成           | |
|  +------------------+  +------------------+  +-----------------+ |
+------------------------------------------------------------------+
                          |
              +-----------+-----------+
              |                       |
     后端服务          外部 API
     (Socket.io 服务器)        (Telegram 等)
```

前端通过两种方式与 **openhuman** Rust 核心通信：用于一小部分壳层命令的 **Tauri IPC**（窗口、AI 文件辅助函数、**`core_rpc_relay`**），以及用于业务逻辑和技能的 **HTTP JSON-RPC**。核心拥有持久连接（如适用）、内存/功能的加密工作，以及 **QuickJS** 沙盒化技能执行。

---

## Rust 驱动的性能

OpenHuman 选择 Tauri + Rust 而非 Electron，基于根本的性能和安全原因：

| 指标 | OpenHuman（Tauri + Rust） | 典型 Electron 应用 |
| ------------------------- | -------------------------------------------------------- | ---------------------------- |
| 二进制体积 | 取决于功能（CEF 运行时 + 技能包占主导） | ~150 MB+ |
| 每技能上下文内存 | ~1-2 MB（QuickJS） | ~150 MB+（Chromium 渲染器） |
| 冷启动 | 亚 500ms | 2-5 秒 |
| 垃圾回收暂停 | 无（Rust 所有权模型） | V8 GC 暂停 |
| 内存安全 | 编译期保证 | 运行时异常 |
| TLS 实现 | rustls（无 OpenSSL 依赖） | Chromium 的 BoringSSL |

**这对加密平台为何重要**：交易员和分析师在运行 OpenHuman 的同时，还会运行资源密集型工具、图表软件、多个浏览器标签、交易终端。原生二进制文件加上亚 500ms 启动意味着应用感觉像原生应用，不会碍事。零 GC 暂停意味着实时价格推送和警报永远不会因内存管理而延迟。

**Tokio 异步运行时**驱动所有 I/O。WebSocket 连接、HTTP 请求、文件操作和技能间通信，都是线程池上的非阻塞任务。数千个并发操作（技能执行、cron job、socket 事件）共享一小套固定的 OS 线程。

---

## 实时 Socket 基础设施

OpenHuman 实现了**双 socket 架构**：桌面端使用 Rust 原生 WebSocket 客户端，Web 端使用 JavaScript Socket.io 客户端。Rust 实现能在应用后台存活，独立于 WebView 运行，并通过 rustls 处理 TLS。

```text
桌面模式：                          Web 模式：

+-------------+                        +-------------+
|  React UI   |                        |  React UI   |
+------+------+                        +------+------+
       | Tauri IPC                          | Direct
+------+------+                        +------+------+
|  Rust Socket |                        |  JS Socket  |
|  Manager     |                        |  .io Client |
+------+------+                        +------+------+
       | tokio-tungstenite                    | Socket.io
       | + rustls TLS                         | (websocket/polling)
+------+------+                        +------+------+
|   Backend   |                        |   Backend   |
+-------------+                        +-------------+
```

**Rust Socket 管理器**通过原始 WebSocket 实现 Engine.IO v4 + Socket.IO v4 帧：

- **握手**：WebSocket 连接、Engine.IO OPEN（提取 `sid`、`pingInterval`、`pingTimeout`）、带 JWT 认证的 Socket.IO CONNECT、CONNECT ACK
- **保活**：响应 Engine.IO PING 以 PONG；超时阈值 = `pingInterval + pingTimeout + 5s`（默认：50 秒）
- **重连**：指数退避，从 1 秒到最大 30 秒。成功连接丢失后重置为 1s；如果连接从未建立则持续增长
- **CORS 绕过**：Rust `reqwest` HTTP 客户端直接发起外部 API 调用，不受浏览器 CORS 限制

socket 连接在所有技能间**共享**。当事件到达时，socket 管理器通过异步消息通道将它们路由到相应的技能。这完全消除了每个技能的连接开销。

**`tool:sync` 协议**：每次 socket 连接和技能生命周期变化时，客户端都会发出一个 `tool:sync` 事件，包含可用工具的完整列表及其连接状态。这使后端 AI 系统能实时感知所有能力。

---

## 技能运行时引擎

OpenHuman 的决定性能力是其运行在 Rust 进程内部的**沙盒化 JavaScript 执行引擎**。技能是轻量级自动化脚本，通过自定义工具、集成和定时任务扩展平台。

```text
+---------------------------------------------------------------+
|                     RuntimeEngine                             |
|                                                               |
|  +-------------------+  +-------------------+                 |
|  | SkillRegistry     |  | CronScheduler     |                |
|  | (HashMap + MPSC)  |  | (5s tick loop)    |                |
|  +--------+----------+  +--------+----------+                |
|           |                      |                            |
|  +--------v----------+  +--------v----------+  +----------+  |
|  | JavaScript Layer  |  | runtime_node      |  |  Bridge  |  |
|  | skill metadata    |  | managed Node.js   |  |   APIs   |  |
|  | + prompt context  |  | system/bundled    |  +----+-----+  |
|  | + tool discovery  |  | tool execution    |       |        |
|  +-------------------+  +-------------------+       |        |
|                                                      |        |
|  +---------------------------------------------------v-----+ |
|  |  net  |  db  |  store  |  cron  |  log  |  tauri  |     | |
|  |  HTTP    SQLite  KV       Schedule  Log    Platform|     | |
|  +------------------------------------------------------+   | |
+---------------------------------------------------------------+
```

**Node.js 运行时**：核心尽可能解析兼容的系统 `node`，否则将托管发行版安装到 OpenHuman 缓存中。技能主要暴露工具元数据，并使用运行时桥接来列出和执行工具，而非在核心内运行隔离的 QuickJS VM。

| 参数 | 值 |
| ---------------------- | ----- |
| 公共语言槽位 | `javascript` |
| 当前 JS 后端 | `runtime_node` |
| 托管 Node 版本 | 默认 `v22.11.0` |
| 运行时来源 | 系统 `node` 或托管安装 |
| 完整性验证 | 针对 `SHASUMS256.txt` 的 SHA-256 |

**工具桥架构**：`SKILL.md` 包提供元数据、指令和可选的捆绑 JS 辅助函数。Rust 核心拥有权威的工具注册表，JavaScript 运行时桥接列出工具并将具名工具调用分派到核心或 Node-backed 辅助函数中。

**桥接 API** 向运行时桥接和 Node-backed 辅助函数暴露平台能力：

| 桥接 | 能力 |
| --------- | ----------------------------------------------------------- |
| **net** | 通过 `reqwest` 的 HTTP fetch（默认 30s 超时，所有方法） |
| **db** | 通过 `rusqlite` 的每个技能 SQLite 数据库 |
| **store** | 键值持久化 |
| **cron** | 定时注册（6 字段 cron 表达式） |
| **log** | 通过 Rust `log` crate 的结构化日志 |
| **tauri** | 平台检测、通知、白名单环境变量 |

**技能发现** 使用 `SKILL.md` 加上可选的捆绑资源：

| 字段 | 用途 |
| ------------------ | ------- |
| `name` | 人类可读的显示名称 |
| `description` | 触发/选择摘要 |
| `metadata.id` | 存在时的稳定技能 slug |
| `allowed-tools` | 工具允许列表指引 |
| 捆绑资源 | 脚本、参考、资源 |

技能从 GitHub 仓库同步并在运行时发现。执行不再建模为每个技能一个嵌入式 QuickJS VM；JavaScript 行为通过共享运行时桥接流动。

**Cron 调度器**：一个 5 秒 tick 循环对照 UTC 时间检查所有已注册的调度，使用 `cron` crate 进行表达式解析。当调度触发时，调度器向技能的通道发送 `CronTrigger` 消息，调用技能的 `onCronTrigger()` 处理程序。

---

## AI & 工具协议（MCP）

OpenHuman 实现了**模型上下文协议**，一个基于 Socket.io 的 JSON-RPC 2.0 层，让 AI 模型发现并由技能暴露的工具。

```text
用户提示
    |
    v
AI 模型（后端）
    |
    |  1. mcp:listTools  -->  前端/Rust 聚合所有技能工具
    |  <-- 工具目录
    |
    |  2. 决定调用哪个工具
    |
    |  3. mcp:toolCall { skillId__toolName, arguments }
    |         |
    |         v
    |     Socket 管理器路由到技能注册表
    |         |
    |         v
    |     QuickJS 技能实例执行工具
    |         |
    |         v
    |     桥接 API 调用（HTTP、DB 等）
    |         |
    |  <-- mcp:toolCallResponse { result }
    |
    v
AI 对用户的响应
```

**传输**：每次请求 30 秒超时，`mcp:` 事件前缀，请求 ID 在待处理响应映射中跟踪。工具名称以 `skillId__toolName` 命名空间化，以实现明确路由。

**工具同步**：`tool:sync` 事件在每次 socket 连接和技能状态变化时广播完整的工具清单、技能 ID、名称、连接状态和工具列表。后端 AI 系统始终拥有可用能力的最新视图。

**AI 记忆系统**：

| 功能 | 实现 |
| ------------------ | ------------------------------------------------------ |
| 静态加密 | 带 Argon2id 密钥派生的 AES-256-GCM |
| 分块 | 每块 512 token，64 token 重叠 |
| 搜索 | 混合：70% 向量相似度 + 30% FTS5 全文 |
| 嵌入 | OpenAI `text-embedding-3-small` |
| 知识图谱 | 通过 REST API 的 Neo4j，用于实体关系 |
| 会话 | 带压缩和工具压缩的 JSONL 转录 |

记忆加密密钥通过 Argon2id 从用户凭证派生，确保记忆文件在未经认证的情况下不可读。混合搜索结合语义理解（向量相似度）和关键词精确度（SQLite FTS5）以实现可靠的召回。

---

## 安全架构

```text
+-------------------------------------------------------------------+
|                      安全层                                       |
|                                                                   |
|  +------------------+  +------------------+  +------------------+ |
|  |  OS 钥匙串       |  |  AES-256-GCM     |  |  沙盒化          | |
|  |  (macOS/Win/Lin) |  |  内存加密        |  |  QuickJS 每      | |
|  |  用于凭证        |  |  + Argon2id KDF  |  |  技能 (64 MB)    | |
|  +------------------+  +------------------+  +------------------+ |
|                                                                   |
|  +------------------+  +------------------+  +------------------+ |
|  |  一次性          |  |  rustls TLS      |  |  无 localStorage | |
|  |  登录 token        |  |  用于所有网络    |  |  存储敏感数据    | |
|  |  (5-min TTL)     |  |  连接            |  |                  | |
|  +------------------+  +------------------+  +------------------+ |
+-------------------------------------------------------------------+
```

- **凭证存储**：通过 `keyring` crate 的 OS 钥匙串集成（macOS Keychain、Windows Credential Manager、Linux Secret Service），仅限桌面端
- **内存加密**：带 Argon2id 密钥派生的 AES-256-GCM。所有 AI 内存静态加密
- **技能沙盒化**：每个 QuickJS 实例都有强制内存限制（默认 64 MB）和栈限制（512 KB）。禁止跨技能内存访问
- **认证交接**：Web 到桌面认证使用 5 分钟 TTL 的一次性登录 token，通过 Rust HTTP 客户端交换（绕过 CORS）
- **网络 TLS**：所有 WebSocket 和 HTTP 连接使用 rustls，不依赖平台 OpenSSL
- **状态管理**：敏感数据保存在 Redux（内存）和 OS 钥匙串（持久化）中。凭证或 token 不使用 localStorage
- **提示注入防护**：用户提示在模型/工具执行前经过规范化/评分，并在服务器端强制执行（`allow | review | block`）。详见 [`docs/PROMPT_INJECTION_GUARD.md`](../../docs/PROMPT_INJECTION_GUARD.md)

---

## 端到端数据流

从用户操作到外部服务再返回的完整流程：

```text
用户在聊天 UI 中输入命令
          |
          v
React 前端分派到 AI 提供商
          |
          v
AI 模型接收提示 + 工具目录（通过 tool:sync）
          |
          v
AI 决定调用技能工具（例如，发送 Telegram 消息）
          |
          v
通过 Socket.io 发送 mcp:toolCall 事件
          |
          v
Socket 管理器（Rust）接收事件，解析 skillId__toolName
          |
          v
技能注册表通过 MPSC 通道将消息路由到正确的 QuickJS 实例
          |
          v
QuickJS 技能执行工具处理程序
          |
          v
桥接 API：net.rs 通过 reqwest 发起 HTTP 请求（无 CORS，rustls TLS）
          |
          v
外部服务响应（例如，Telegram API）
          |
          v
结果回流：桥接 -> QuickJS -> 注册表 -> Socket -> MCP -> AI -> UI
          |
          v
用户在聊天界面中看到结果
```

每一层都是异步且非阻塞的。Rust 核心在固定的 Tokio 线程池上处理数千个并发的技能执行、cron 触发和 socket 事件。

---

## 技术栈

| 层 | 技术 | 原因 |
| -------------- | ------------------------------- | -------------------------------------------------------- |
| **前端** | React 19, TypeScript 5.8 | 现代组件模型，类型安全 |
| **状态** | Redux Toolkit + Persist | 可预测状态，支持离线持久化 |
| **构建** | Vite 7 | 亚秒级 HMR，优化的生产构建 |
| **样式** | Tailwind CSS | 工具优先，一致的设计系统 |
| **框架** | Tauri v2 | 原生跨平台，开销最小 |
| **语言** | Rust (2021 edition) | 内存安全，零成本抽象 |
| **异步** | Tokio | 高性能异步 I/O 运行时 |
| **JS 运行时** | Node.js | 用于工具辅助函数和技能相关 JS 的托管 V8 运行时 |
| **数据库** | SQLite (rusqlite) | 嵌入式，零配置，每技能隔离 |
| **WebSocket** | tokio-tungstenite + rustls | 持久连接，原生 TLS |
| **HTTP** | reqwest | 异步 HTTP，支持 rustls + native-tLS 双栈 |
| **加密** | aes-gcm + argon2 | AES-256-GCM 加密，Argon2id 密钥派生 |
| **调度** | cron crate + 自定义调度器 | 标准 cron 表达式，5 秒精度 |
| **Telegram** | 已移除 | Telegram 集成已移除 |
| **实时** | Socket.io（客户端） | 双向基于事件的通信 |
| **AI** | MCP（JSON-RPC 2.0） | LLM 集成的标准化工具协议 |
| **搜索** | OpenAI 嵌入 + SQLite FTS5 | 混合语义 + 关键词搜索 |
| **图谱** | Neo4j | 实体关系知识图谱 |
