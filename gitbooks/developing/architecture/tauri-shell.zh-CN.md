---
description: 桌面宿主 (`app/src-tauri/`) —— Tauri v2 + WebView、IPC、嵌入式核心生命周期、核心桥接。
icon: desktop
---

# Tauri Shell (`app/src-tauri/`)

OpenHuman 的桌面宿主：Tauri v2 + WebView、IPC 命令、窗口管理，以及桥接到嵌入式 `openhuman-core` Rust 运行时（核心 JSON-RPC）。它**不会**重复完整的领域栈；那部分存在于仓库根目录的 Rust crate 中（`openhuman_core`、`src/main.rs`）。

## 职责

1. **Web UI**。从 `app/dist` 加载 Vite 构建（或开发服务器，端口 1420）。
2. **IPC**。暴露一小套明确的 Tauri 命令（见 [Commands](#tauri-ipc-commands-app-src-tauri)）。
3. **核心生命周期**。启动进程内核心服务器，并通过 `core_rpc_relay` 代理 JSON-RPC。
4. **磁盘上的 AI 提示**。从资源 / 开发 cwd 解析捆绑的 `src/openhuman/agent/prompts`，用于 `ai_get_config` / `write_ai_config_file`。
5. **窗口 + 托盘**。桌面窗口行为和系统托盘（见 `lib.rs`）。

## 核心进程模型

`app/package.json` 的 `core:stage` 现在有意保持为 no-op，仅用于脚本兼容性。桌面应用会在进程内链接核心，因此本地构建不再需要在 `app/src-tauri/binaries/` 下 staging `openhuman-core-*` sidecar。

## 卡死进程恢复

正常应用退出从 `RunEvent::ExitRequested` 运行 teardown：CEF 关闭前先关闭子 webview，触发嵌入式核心的 cancellation token，最终进程扫描在短暂的宽限期后向直接子进程发送 `SIGTERM`，然后升级使用 `SIGKILL` 处理顽固进程。扫描摘要记录为 `[app] sweep: term=N kill=M total=K`；任何非零 `kill` 计数都是警告，意味着子进程忽略了优雅关闭。

在 macOS 上，硬退出（强制退出、`SIGKILL`、渲染器崩溃）可能跳过正常的 teardown。下一次启动在 CEF 缓存 preflight 之前运行启动恢复：它列出可执行路径属于正在启动的 `.app/Contents` 的 OpenHuman 进程，跳过当前进程，发送 `SIGTERM`，短暂等待，然后对仍然匹配相同 pid+command 的顽固进程发送 `SIGKILL`。日志使用 `[startup-recovery]` 前缀。

当设置了 `OPENHUMAN_CORE_REUSE_EXISTING=1` 时（以便手动 CLI-core 复用仍然有效），以及当 CEF `SingletonLock` 被实时进程持有时（以便正常的 second-instance 路径可以在不杀死已运行应用的情况下失败），启动恢复跳过。Tauri 命令 `process_diagnostics_list_owned` 返回当前拥有的进程列表；macOS 实现是 bundle 作用域的，Linux/Windows 目前返回空。


## Tauri Shell 架构 (`app/src-tauri/`)

### 概述

**`app/src-tauri`** crate（Rust 包 **`OpenHuman`**，二进制文件 **`OpenHuman`**）是一个**仅限桌面**的宿主。它嵌入 React UI，注册插件（深度链接、打开器、OS、通知、自动启动、更新器），管理主窗口和托盘，并**中继 JSON-RPC** 到嵌入式核心服务器。

非桌面目标在编译时失败（`lib.rs` 中的 `compile_error!`）。

### 目录布局（实际）

```text
app/src-tauri/src/
├── lib.rs                 # `run()`、托盘/菜单动作、插件、`generate_handler!`、核心启动
├── main.rs                # 二进制入口
├── core_process.rs        # CoreProcessHandle、嵌入式核心服务器任务
├── core_rpc.rs            # 核心 JSON-RPC 的 HTTP 客户端
├── commands/
│   ├── mod.rs             # 重新导出
│   ├── core_relay.rs      # `core_rpc_relay`、服务管理的核心引导
│   ├── openhuman.rs       # Daemon 宿主配置、systemd 风格服务辅助函数
│   └── window.rs          # 显示/隐藏/最小化/关闭窗口
└── utils/
    ├── mod.rs
    └── dev_paths.rs       # 解析捆绑的 AI 提示路径
```

此树中**没有** `src-tauri/src/services/session_service.rs`；会话语义在 Web 层 + 后端 + 核心中按适用情况处理。

### 数据流：UI → 核心

```text
React (invoke)
    → core_rpc_relay { method, params, serviceManaged? }
        → core_rpc::call HTTP POST 到 OPENHUMAN_CORE_RPC_URL
            → 嵌入式 openhuman 核心服务器
```

`core_process.rs` 中的 `CoreProcessHandle` 拥有嵌入式服务器任务；`commands/core_relay.rs` 可选地在 relay 之前确保**服务管理**的核心正在运行。

### 窗口和托盘行为

- 壳层在启动时创建托盘图标，并将动作连接到打开主窗口或退出。
- 在 daemon 模式（`daemon` / `--daemon`）下，主窗口在启动时隐藏，可以从托盘动作重新打开。
- 在 macOS 上，`RunEvent::Reopen` 也会恢复并聚焦主窗口。
- Windows 和 Linux 使用相同的托盘动作（`Open OpenHuman`、`Quit`），某些 Linux 设置上有桌面环境特定的托盘渲染差异。

### 捆绑资源

`tauri.conf.json` 捆绑 **`../../skills/skills`** 和 **`../../src/openhuman/agent/prompts`**，使技能和提示 markdown 随应用一起发布。

### 相关

- IPC 表面：见下方的 [Commands](#tauri-ipc-commands-app-src-tauri) 部分
- HTTP 桥接：见下方的 [Core bridge & helpers](#core-bridge-helpers-app-src-tauri) 部分
- Rust 领域（实现）：仓库根目录 `src/openhuman/`、`src/core_server/`


## Tauri IPC 命令 (`app/src-tauri`) {#tauri-ipc-commands-app-src-tauri}

所有命令都在 **`app/src-tauri/src/lib.rs`** 中的 `tauri::generate_handler![...]` 内注册（桌面构建）。下方名称是 **Rust** 命令名称（在 JS 中通过 serde 应用 camelCase）。

### Demo / 诊断

| 命令 | 用途 |
| ------- | ------------------------------------------ |
| `greet` | Demo 字符串（生产中可安全移除） |

### AI 配置（捆绑提示）

| 命令 | 用途 |
| ---------------------- | -------------------------------------------------------------------------------------------- |
| `ai_get_config` | 从捆绑或开发 `src/openhuman/agent/prompts` 下解析的 `SOUL.md` / `TOOLS.md` 构建 `AIPreview` |
| `ai_refresh_config` | 与 `ai_get_config` 相同的读取路径（刷新 hook） |
| `write_ai_config_file` | 在仓库 `src/openhuman/agent/prompts` 下写入单个 `.md`（开发 / 安全文件名检查） |

### 核心 JSON-RPC 中继

| 命令 | 用途 |
| ---------------- | -------------------------------------------------------------------------------------------------------------- |
| `core_rpc_relay` | Body: `{ method, params?, serviceManaged? }` → 转发到本地 **`openhuman-core`** HTTP JSON-RPC (`core_rpc.rs`) |

从前端使用 **`app/src/services/coreRpcClient.ts`** (`callCoreRpc`)。

### 窗口管理

来自 **`commands/window.rs`**（名称可能略有不同；见 `lib.rs`）：

| 命令 | 用途 |
| ------------------- | ----------------- |
| `show_window` | 显示主窗口 |
| `hide_window` | 隐藏主窗口 |
| `toggle_window` | 切换可见性 |
| `is_window_visible` | 查询可见性 |
| `minimize_window` | 最小化 |
| `maximize_window` | 最大化 |
| `close_window` | 关闭 |
| `set_window_title` | 设置标题字符串 |

### OpenHuman daemon / 服务辅助函数

来自 **`commands/openhuman.rs`**（见源码获取精确 payload）：

| 命令 | 用途 |
| ---------------------------------- | ---------------------------------------------- |
| `openhuman_get_daemon_host_config` | 读取 daemon 宿主偏好设置（例如托盘） |
| `openhuman_set_daemon_host_config` | 持久化 daemon 宿主偏好设置 |
| `openhuman_service_install` | 安装后台服务（平台特定） |
| `openhuman_service_start` | 启动服务 |
| `openhuman_service_stop` | 停止服务 |
| `openhuman_service_status` | 查询状态 |
| `openhuman_service_uninstall` | 卸载服务 |

### 屏幕共享选择器（CEF / macOS）

来自 **`screen_capture/mod.rs`**。支持 `webview_accounts/runtime.js` 中的页面内 `getDisplayMedia` shim。会话门控：shim 必须在成功枚举/缩略图捕获之前用实时用户手势打开会话。见 issue #713（选择器 UX）+ #812（会话门控）。

| 命令 | 用途 |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `screen_share_begin_session` | 从账户 webview 打开 30s 会话，在 `navigator.userActivation.isActive` 手势之后。返回 `{ token, sources }`。每个账户限速 10/分钟。 |
| `screen_share_thumbnail` | 将单个来源的缩略图捕获为 base64 PNG。需要 live token 和会话颁发的 `id`。仅 macOS；其他平台返回错误。 |
| `screen_share_finalize_session` | 关闭会话。由 shim 在 Share 或 Cancel 时调用；使用未知/过期 token 安全调用（no-op）。 |

### 已移除 / 不存在

以下命令**不**存在于当前的 `generate_handler!` 列表中：`exchange_token`、`get_auth_state`、`socket_connect`、`start_telegram_login`。认证和 socket 在 **React** 应用和 **核心** 进程中处理，而非通过这些 IPC 名称。

### 示例：核心 RPC

```typescript
import { invoke } from "@tauri-apps/api/core";

const result = await invoke("core_rpc_relay", {
  request: {
    method: "your.rpc.method",
    params: { foo: "bar" },
    serviceManaged: false,
  },
});
```

---

_见 `app/src-tauri/src/lib.rs` 获取权威列表。_


## Core bridge & helpers (`app/src-tauri`) {#core-bridge-helpers-app-src-tauri}

本文档替代了旧的 "SessionService / SocketService" 拆分。Tauri crate **不**嵌入重复的 Socket.io 服务器或 Telegram 客户端；相反，它专注于对 **`openhuman-core`** 二进制文件的**进程管理**和 **HTTP JSON-RPC**。

### `CoreProcessHandle` (`core_process.rs`)

- 解析 **`openhuman-core`** 可执行文件（staging 在 `binaries/` 下或 `PATH` / 开发布局中）。
- 启动或附加到核心进程并暴露其 RPC URL (`OPENHUMAN_CORE_RPC_URL`)。
- 在 `lib.rs` 的应用设置期间使用 (`app.manage(core_handle)`)。

### `core_rpc` (`core_rpc.rs`)

- 核心 JSON-RPC 表面的 HTTP 客户端（localhost）。
- 由 **`core_rpc_relay`** 使用，以转发前端的 `method` + `params`。

### `commands/core_relay.rs`

- **`core_rpc_relay`**。确保核心正在运行（进程内句柄或**服务管理**路径），然后调用 `core_rpc`。
- **`ensure_service_managed_core_running`**。当 RPC 不可用时引导 systemd/launchd 风格服务（核心 CLI 内的平台特定行为）。

### `commands/openhuman.rs`

- Daemon 宿主 JSON 配置（例如托盘可见性），位于应用数据目录下。
- 为 **openhuman** 后台服务提供 install/start/stop/status/uninstall 辅助函数。

### `utils/dev_paths.rs`

- 解析 AI preview 的开发和捆绑资源路径下的 **`src/openhuman/agent/prompts`**。

### `utils/tauriSocket.ts`（前端）

不在 `src-tauri` 中，但与 shell **配对**：React 应用监听镜像 Rust 端客户端 socket 活动的 Tauri 事件。见 `app/src/utils/tauriSocket.ts` 和 [前端服务](frontend.zh-CN.md#services-layer) 章节。

---
