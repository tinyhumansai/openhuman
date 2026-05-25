---
description: >-
  为什么 OpenHuman 自带 Chromium 运行时，我们今天用它做什么，以及同样的 CDP 表面接下来能解锁什么。
icon: chrome
---

# Chromium Embedded Framework

OpenHuman 不运行在平台内置的 webview 上。它通过 `tauri-runtime` 的一个 fork 自带 **Chromium Embedded Framework (CEF) 运行时**，而这一个决策对产品几乎所有 "OpenHuman 知道你的工具里发生了什么" 的功能都是 load-bearing 的。

本页解释为什么 CEF 在 bundle 中，代码库今天用它做什么，以及同样的表面可以去哪里。

## 为什么用 CEF 而不是 stock webview

Stock Tauri 使用每个平台的原生 webview。macOS 上的 WKWebView、Windows 上的 WebView2、Linux 上的 WebKitGTK。这些用于渲染 OpenHuman 应用本身都能正常工作。它们对我们的用例有一个致命的局限性：**没有一个暴露 Chrome DevTools Protocol (CDP)**。

CDP 是 load-bearing 的原语。OpenHuman 中每个 "观察 Slack / WhatsApp / Telegram / Discord / Meet 内部发生了什么" 的功能都通过 CDP 与这些嵌入应用对话，而非通过注入的 JavaScript。CDP 提供：

* `Target.getTargets` 用于发现每个页面和服务 worker。
* `IndexedDB.requestDatabaseNames` / `requestDatabase` / `requestData` 用于遍历第三方应用的本地存储。
* `DOMSnapshot.captureSnapshot` 用于不会触发框架反应性的只读 DOM 检查。
* `Runtime.evaluate` 用于短暂的一次性读取（单个固定的 JSON 序列化器，从来不是持久桥接）。
* `Page.addScriptToEvaluateOnNewDocument` 用于极少数我们真正需要在页面 JS 运行前渲染器端 shim 的情况。

Stock webview 不能给我们任何这些。所以我们 vendor CEF。

Vendored 运行时位于 [`app/src-tauri/vendor/tauri-cef/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src-tauri/vendor/tauri-cef)（从上游 `tauri-cef` 分支 fork 到 `tinyhumansai/tauri-cef:feat/cef-notification-intercept`，当前 CEF 146.4.1）。每个 Tauri crate 在 `app/src-tauri/Cargo.toml` 中通过 `[patch.crates-io]` 指向此 fork。Vendored `cargo-tauri` CLI 将 Chromium 正确捆绑到 `Contents/Frameworks/`；stock `@tauri-apps/cli` 会产生一个损坏的 bundle，在 `cef::library_loader::LibraryLoader::new` 中 panic。[`scripts/ensure-tauri-cli.sh`](../../scripts/ensure-tauri-cli.sh) 在 fork 比安装的二进制文件更新时重新安装 vendored CLI。

## CEF 今天用于什么

### 嵌入的第三方 webview

每个作为托管 Web 应用运行的已连接提供商都有自己的子 CEF webview：

* WhatsApp Web
* Telegram Web
* Slack
* Discord
* Google Meet
* LinkedIn
* Gmail
* Zoom
* browserscan

每个账户的存储隔离到 `{app_local_data_dir}/webview_accounts/{id}/`。两个 Slack workspace，两个浏览器配置文件。代码：[`app/src-tauri/src/webview_accounts/mod.rs`](../../app/src-tauri/src/webview_accounts/mod.rs)。

### CDP 驱动的扫描器

每个提供商在 [`app/src-tauri/src/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src-tauri/src) 中都有一个**扫描器模块**。每个扫描器持有到 CEF 的 `--remote-debugging-port=19222` 的长期 WebSocket，并按固定节奏 tick：

| 扫描器 | 节奏 | 做什么 |
| ------------------ | ------------------------------- | -------------------------------------------------------------------- |
| `whatsapp_scanner` | 2s DOM tick + 30s 完整 IDB 遍历 | 读取消息存储、拉取媒体元数据 |
| `telegram_scanner` | 相同 | 额外加上 QR 登录 hand-off 到原生 Telegram Desktop |
| `slack_scanner` | 30s IDB 遍历 | 纯 IDB —— 无需 DOM 抓取 |
| `discord_scanner` | 定期 | 通过 CDP 的频道 + DM 状态 |
| `meet_scanner` | 定期 | 通话期间的实时字幕 + 参与者状态 |
| `imessage_scanner` | 定期 | **无 webview。** 在 macOS 上直接读取 `~/Library/Messages/chat.db` |

每次扫描都会发出 `webview:event` payload，并直接向核心 RPC POST `openhuman.memory_doc_ingest`，因此无论 UI 窗口是否打开或后台运行，记忆都会增长。

### Google Meet mascot 摄像头

最炫的 CEF 技巧。Meet Agent 不只是"参加会议"，它还**将自己广播为摄像头**。之所以能工作，是因为 CEF 允许我们：

1. 在任何 Meet 代码运行前通过 `Page.addScriptToEvaluateOnNewDocument` 注入一个微小桥接 (`camera_bridge.js`)。
2. 覆盖 `navigator.mediaDevices.getUserMedia`，使其从隐藏的 640×480 canvas 返回 `MediaStream`，而非真实摄像头。
3. 在该 canvas 上渲染 mascot SVG，通过 Rust 经 CDP 驱动的 `window.__openhumanSetMood(...)` 交换情绪状态（idle、thinking、talking）。

还有一个构建时路径，将 mascot SVG 栅格化为 Y4M，并使用 CEF 的原生 `--use-file-for-fake-video-capture` flag，一个完全原生的 fake-camera 来源，完全不使用 JS。

代码：[`app/src-tauri/src/meet_video/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src-tauri/src/meet_video)。

### 原生通知拦截

`feat/cef-notification-intercept` 上的 fork 为 `Notification.permission`、`Notification.requestPermission()` 和 `navigator.permissions.query({name: "notifications"})` 添加了渲染器端 shim。这些现在在每条运行时代码路径上都安装在真正的 `tauri-runtime-cef` 路径中，因此当 Slack 检查它是否可以显示通知时，答案与 CEF 的权限回调已经授予的内容一致。

这是 `docs/TAURI_CEF_FINDINGS_AND_CHANGES.md` 的大部分内容。这就是 Slack 在一次会话中不再五次询问相同权限的原因。

## "不注入新 JS" 规则

规则记录在 [`CLAUDE.md`](../../CLAUDE.md) 中：**迁移的提供商以零注入 JavaScript 加载**。所有抓取都通过扫描器侧的 CDP 原生进行。

这很重要，因为任何在第三方来源内部运行的宿主控制代码都是攻击面责任。Slack 内部的持久 JS 桥接离失效只有一个 Slack 更新之遥，离通过攻击者控制的 JS 泄露桥接只有一个错误之遥。从渲染器外部的 CDP 严格更好。

| 提供商 | 已迁移？ | 启动时加载什么 |
| ----------- | ------------- | -------------------------------- |
| WhatsApp | ✅ | 零 JS |
| Telegram | ✅ | 零 JS |
| Slack | ✅ | 零 JS |
| Discord | ✅ | 零 JS |
| browserscan | ✅ | 零 JS |
| Gmail | grandfathered | 遗留 `runtime.js` 桥接 |
| LinkedIn | grandfathered | 遗留 `LINKEDIN_RECIPE_JS` |
| Google Meet | grandfathered | 摄像头 + 音频 + 字幕桥接 |

遗留注入应该缩小，永远不要增长。新提供商直接走 CDP-only 路径。

## CEF 预热

一个隐藏的 CEF webview (`cef-prewarm`) 在应用启动时启动浏览器，因此当用户点击时第一个子 webview 立即生成。它在 `cef::shutdown()` 前被拆除以避免退出时的竞争。见 `app/src-tauri/src/lib.rs` 中 prewarm + 关闭生命周期附近的代码。

## Windows 启动诊断

CEF 在 onboarding UI 能够从渲染器故障中恢复之前初始化。如果 Windows 用户报告静默退出、永久的 "Connecting..." 转圈，或在第一个交互窗口出现前的 `tauri-runtime-cef` 断言，请在 issue 中询问这些细节：

* Windows 版本和完整构建号，特别是 Insider 构建。
* OpenHuman 版本和安装包类型（`.msi` 或 `.exe`）。
* 重试前是否将 `%LOCALAPPDATA%\com.openhuman.app` 移到了一边。
* `[startup]`、`[cef-profile]` 和 `[cef-startup]` 的启动日志行。
* 任何命名 `tauri-runtime-cef/src/lib.rs` 的 panic 文本。

对于 Windows Insider 构建，还要确认相同的安装包是否在当前稳定版 Windows 发布上启动。这会将 profile/缓存问题与 CEF 启动中的 OS/运行时兼容性回归分开。

## Linux shell fallback（CEF 启动崩溃时）

在某些 Linux 桌面上，特别是 NVIDIA 专有驱动设置下的 Wayland/XWayland，Tauri/CEF shell 可能在 React 应用变得可用之前的原生窗口配置期间失败。一个已知症状是 CEF 报告主浏览器上下文后的 X11 `BadWindow` 错误。

当核心本身健康时，你可以通过分别运行核心和前端来继续开发：

```bash
cargo build --bin openhuman-core
./target/debug/openhuman-core run --port 7788
```

在另一个终端：

```bash
cd app
pnpm dev
```

在常规浏览器中打开 Vite URL，选择 **Advanced** / remote core 模式，将 RPC URL 设置为 `http://127.0.0.1:7788/rpc`，并使用核心写入的 bearer token。这会绕过原生专属功能，如托盘、自动更新和嵌入提供商 webview，但保持智能体、记忆、技能和 RPC 表面可用于调试。

## 插件审计

添加到 `app/src-tauri/src/lib.rs` 的任何新内容都必须审计 `js_init_script` 调用。`tauri-plugin-opener` 默认附带一个 init 脚本 (`init-iife.js`)，添加了一个全局点击监听器；我们将其配置为 `.open_js_links_on_click(false)`，使其不在第三方 webview 内运行。`tauri-plugin-notification` 的 init 脚本同样从 vendored 副本中删除。

## 这里可以如何演进

CDP 表面是通用的。今天它为固定列表的提供商提供记忆摄入；同样的原语可以做更多。

### 浏览器自动化作为一等智能体工具

今天智能体有[原生工具](../features/native-tools/README.zh-CN.md)用于文件系统、git、网页搜索和网页获取。下一个明显的工具是**"驱动真实浏览器会话"**：登录用户已认证过的 SaaS，填写表单，抓取分页表格，下载导出。

 plumbing 已经存在。`@openhuman/browser_task` 技能可以启动一个专用 CEF webview，通过 CDP 从核心驱动它，并将结果作为工具调用展示。用户现有的每账户配置文件意味着无需重新认证。

### Headless CEF 用于服务端回放

同样的扫描器模式（长期 WebSocket → IDB 遍历 + DOM snapshot）无需 UI 即可工作。核心 sidecar 中的 Headless CEF 可以按计划回放会话，适用于在云端托管核心并希望从不暴露干净 OAuth API 的来源自动获取的用户。

### 浏览器进程层的隐私 hook

CEF 的 `CefRequestHandler` 已经允许我们拦截网络请求。从"拦截并记录"到"拦截并重写"只有一小步：广告拦截、跟踪器拦截、每个提供商的 DNS 固定、请求重写。隐私作为一等浏览器功能，而非每个来源内泄漏的 JS shim。

### CDP 驱动的测试框架

扫描器模式、生成 webview、遍历 IDB、snapshot DOM、评估一个短暂表达式，在结构上与 E2E 测试编排相同。我们可以将 `@openhuman/web_test` 作为公共技能发布：`connect_cef → snapshot → evaluate → assert`。用纯 Rust 针对任何 Web 应用编写的测试，无需 Selenium / Playwright 依赖。

### 渲染器 ↔ Rust 消息通道

今天每个 CDP `Runtime.evaluate` 都是 fire-and-forget。从渲染器到 Rust 的长期双向通道（Tauri 为主机应用做 IPC 的方式）将解锁流式用例：实时打字检测、实时选择/高亮跟踪、主动推送。设计它时不违反"第三方来源中不允许持久 JS 桥接"规则是有趣的约束。

### 多账户合并

每个连接账户都有自己的配置文件和自己的 IDB。CDP 可以 snapshot 一个账户的 IDB，与另一个账户的解密合并，并 upsert 到共享的记忆文档中，例如跨三个 workspace 的统一 Slack 记忆。

## 另请参阅

* [`docs/TAURI_CEF_FINDINGS_AND_CHANGES.md`](../../docs/TAURI_CEF_FINDINGS_AND_CHANGES.md)。通知权限深度解析。
* [`CLAUDE.md`](../../CLAUDE.md)。权威的"不注入新 JS"规则。
