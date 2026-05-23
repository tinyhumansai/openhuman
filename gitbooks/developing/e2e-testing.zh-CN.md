---
description: 使用 WDIO + tauri-driver / Appium 进行端到端测试。CI 和本地设置。
icon: vials
lang: zh-CN
---

# E2E 测试指南

## 概述

桌面 E2E 测试使用 **WebDriverIO (WDIO)** 通过两个自动化后端驱动 Tauri 应用：

| 平台 | 驱动 | 端口 | 应用格式 | 选择器 |
|----------|--------|------|------------|-----------|
| **Linux / CEF 状态** | `tauri-driver` | 4444 | Debug 二进制文件 | CSS / DOM |
| **macOS / Appium** | Appium Mac2 | 4723 | `.app` 包 | XPath / 辅助功能 |

OpenHuman 桌面应用目前使用 CEF 运行时（`tauri-runtime-cef`）。Linux `tauri-driver` 与 WebKitWebDriver / webkit2gtk 通信，无法驱动 CEF -backed WebView，因此 Linux CEF E2E 在 CI 中被禁用，直到存在 CEF 兼容的驱动或替代 harness。目前支持的路径是 macOS/Appium 用于本地运行，以及在该工作流启用时手动触发 macOS/Appium 工作流运行。

---

## 快速开始

### Linux / CEF 状态

```bash
# 安装 tauri-driver（一次性）
cargo install tauri-driver

# 构建 E2E 应用
pnpm --filter openhuman-app test:e2e:build

# 运行所有流程
pnpm --filter openhuman-app test:e2e:all:flows

# 运行单个 spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

在无头 Linux 上，harness 在 **Xvfb** 虚拟显示下运行。此路径目前仅对非 CEF / WebKit 兼容调试有用；默认 CEF 应用无法被 WebKitWebDriver 自动化。

### macOS / Appium

```bash
# 安装 Appium + Mac2 驱动（一次性，需要 Node 24+）
npm install -g appium
appium driver install mac2

# 构建 .app 包
pnpm --filter openhuman-app test:e2e:build

# 运行所有流程
pnpm --filter openhuman-app test:e2e:all:flows
```

### macOS 上的 Docker（本地运行 Linux harness）

使用 Docker 从 macOS 运行相同的基于 Linux 的 harness。同样的 CEF 限制适用：在存在 CEF 兼容驱动之前，这不是默认 CEF 运行时的支持路径。

```bash
# 构建 + 运行所有 E2E 流程
docker compose -f e2e/docker-compose.yml run --rm e2e

# 先构建应用（如需要）
docker compose -f e2e/docker-compose.yml run --rm e2e \
  pnpm --filter openhuman-app test:e2e:build

# 运行单个 spec
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

需要 Docker Desktop 或 Colima。仓库通过 bind mount 挂载，因此构建在运行之间持久化。

---

## 架构

### 平台检测

`app/test/e2e/helpers/platform.ts` 导出：

- `isTauriDriver()`，`true` 表示 Linux（tauri-driver session）
- `isMac2()`，`true` 表示 macOS（Appium Mac2 session）
- `supportsExecuteScript()`，`true` 当 `browser.execute()` 可用时（仅 tauri-driver）

### 元素辅助函数

`app/test/e2e/helpers/element-helpers.ts` 提供统一 API：

| 辅助函数 | Mac2 (macOS) | tauri-driver (Linux) |
|--------|-------------|---------------------|
| `waitForText(text)` | @label/@value/@title 上的 XPath | DOM 文本内容上的 XPath |
| `waitForButton(text)` | XCUIElementTypeButton XPath | `button` / `[role="button"]` XPath |
| `clickText(text)` | W3C 指针动作 | 标准 `el.click()` |
| `clickNativeButton(text)` | XCUIElementTypeButton 上的 W3C 指针动作 | button 上的标准 `el.click()` |
| `clickToggle()` | XCUIElementTypeSwitch / XCUIElementTypeCheckBox | `[role="switch"]` / `input[type="checkbox"]` |
| `waitForWindowVisible()` | XCUIElementTypeWindow | 窗口句柄检查 |
| `waitForWebView()` | XCUIElementTypeWebView | `document.readyState` 检查 |
| `hasAppChrome()` | XCUIElementTypeMenuBar | 窗口句柄检查 |
| `dumpAccessibilityTree()` | 辅助功能 XML | HTML 页面源码 |

### 稳定的测试 ID

优先为 E2E spec 点击或轮询的 UI  affordance 使用稳定的 `data-testid` hook。使用分类法 `<surface>-<element>-<id?>`，例如：

- `cron-jobs-panel`、`cron-refresh`
- `cron-job-row-<jobId>`、`cron-job-toggle-<jobId>`、`cron-job-run-<jobId>`、`cron-job-view-runs-<jobId>`、`cron-job-remove-<jobId>`
- `settings-nav-<routeId>`
- `skill-row-<skillId>`、`skill-install-<skillId>`、`skill-uninstall-<skillId>`
- `thread-row-<threadId>`、`new-thread-button`、`send-message-button`
- `onboarding-next-button`

当 spec 瞄准这些 hook 之一时，使用 `element-helpers.ts` 中的 `waitForTestId(testId)` 和 `clickTestId(testId)`。对行/动作发现保留文本选择器，对用户可见文案断言也保留文本选择器。

### 深度链接辅助函数

`app/test/e2e/helpers/deep-link-helpers.ts` 处理 auth 深度链接：

- **tauri-driver**：`browser.execute(window.__simulateDeepLink(url))`（主要），`xdg-open`（备用）
- **Appium Mac2**：`macos: deepLink` 扩展命令（主要），`open -a ...`（备用）

对于发布候选版，在触碰 CEF preflight、单实例或深度链接启动代码时，还要在 Linux 或 macOS 上运行一次手动 secondary-instance 冒烟测试：

1. 正常启动 OpenHuman 并保持运行。
2. 通过 OS opener 触发 `openhuman://auth?token=e2e-token&key=auth`。
3. 确认已运行的窗口接收到回调，且不会启动第二个完整的 CEF 实例。
4. 确认 secondary 进程干净退出，没有 CEF 缓存锁错误。

这捕捉了一类回归：secondary 进程在 Tauri 的深度链接转发路径安装之前，于 CEF 缓存 preflight 期间退出。

### 编写跨平台 spec

1. 在 spec 中使用 `element-helpers.ts` 中的**辅助函数**，永远不要使用原始的 `XCUIElementType*` 选择器
2. 使用 **`clickNativeButton(text)`** 代替内联 button-clicking 代码
3. 使用 **`hasAppChrome()`** 代替检查 `XCUIElementTypeMenuBar`
4. 使用 **`waitForWebView()`** 代替检查 `XCUIElementTypeWebView`
5. 对于仅 macOS 的测试，使用 `process.platform` 守卫或单独的 spec 文件
6. 对 hash 路由使用 `navigateViaHash(route)`；它等待 hash、`document.readyState` 和挂载的 React root 后返回。在 onboarding 之后，`walkOnboarding()` 也等待 `#/home` 加上 Home 页面标记，然后 spec 才会导航到别处。

---

## 环境变量

| 变量 | 默认值 | 说明 |
|----------|---------|-------------|
| `TAURI_DRIVER_PORT` | `4444` | tauri-driver WebDriver 端口 |
| `APPIUM_PORT` | `4723` | Appium 服务器端口 |
| `E2E_MOCK_PORT` | `18473` | Mock 后端服务器端口 |
| `OPENHUMAN_WORKSPACE` | (临时目录) | 应用工作区目录 |
| `OPENHUMAN_SERVICE_MOCK` | `0` | 启用服务 mock 模式 |
| `OPENHUMAN_E2E_MODE` | 未设置 | 启用破坏性测试支持 RPC；E2E runner 将其设为 `1` |
| `OPENHUMAN_E2E_AUTH_BYPASS` | 未设置 | 启用 JWT 绕过认证 |
| `DEBUG_E2E_DEEPLINK` | (verbose) | 设为 `0` 以静默深度链接日志 |
| `E2E_FORCE_CARGO_CLEAN` | 未设置 | E2E 构建前强制 cargo clean |

---

## CI 工作流

### Push / PR 检查

默认的 `test.yml` 工作流运行前端单元测试和 Rust 检查。其 Linux `tauri-driver` E2E job 被注释掉了，因为 WebKitWebDriver 无法驱动 CEF-backed WebView。

被禁用的 Linux E2E job 过去会：
1. 安装系统依赖（webkit2gtk、Xvfb、dbus）
2. 通过 cargo 安装 `tauri-driver`
3. 用 mock 服务器 URL 构建应用
4. 在 Xvfb 下运行所有 E2E 流程

### macOS / Appium

macOS/Appium 是当前 CEF 桌面应用支持的自动化后端。在本地运行，或在该工作流启用时通过手动触发的 macOS 工作流运行：
1. 安装 Appium + Mac2 驱动
2. 构建 `.app` 包
3. 运行所有 E2E 流程

---

## 故障排除

### Linux："WebView not ready" 超时

对于默认 CEF 运行时，这通常意味着不支持的 Linux `tauri-driver` 路径正试图通过 WebKitWebDriver 驱动 CEF-backed WebView。请使用 macOS/Appium，或等待 CEF 兼容的 Linux 驱动。

确保 `DISPLAY` 已设置且 Xvfb 正在运行：
```bash
export DISPLAY=:99
Xvfb :99 -screen 0 1280x1024x24 &
```

还要确保 dbus 已启动（webkit2gtk 需要）：
```bash
eval $(dbus-launch --sh-syntax)
```

### Linux：找不到 tauri-driver

```bash
cargo install tauri-driver
```

### macOS：深度链接在 `tauri dev` 中不工作

深度链接需要 `.app` 包。请改用 `pnpm tauri build --debug --bundles app`。

### Docker：首次运行构建很慢

首次 Docker 构建从源码编译 Rust + tauri-driver。后续运行使用缓存层。Cargo registry 和 git 源通过 Docker volume 缓存。

## Spec：Notifications

**文件**：`app/test/e2e/specs/notifications.spec.ts`

通过实时 core sidecar 和 Notifications UI 页面测试 notification RPC 方法：

- `notification_ingest`，通过 core RPC 创建新通知
- `notification_list`，验证摄入的通知被返回
- `notification_mark_read`，将通知标记为已读
- `notification_stats`，检查聚合统计形状
- UI：Notifications 页面渲染集成通知部分（`[data-testid="integration-notifications-section"]`）
- UI：Notifications 页面显示 System Events 部分（`[data-testid="system-events-section"]`）

**运行**：

```bash
bash app/scripts/e2e-run-spec.sh test/e2e/specs/notifications.spec.ts notifications
```

**平台说明**：RPC 测试（`notification_ingest`、`notification_list`、`notification_mark_read`、`notification_stats`）为 Linux/tauri-driver 和 macOS/Appium Mac2 编写，但默认 CEF 运行时的 Linux 执行被禁用，直到存在 CEF 兼容驱动。UI 断言（Notifications 页面部分）需要 `browser.execute()` 支持，因此当 `supportsExecuteScript()` 返回 `false` 时，它们在 Mac2 上自动跳过。

---

## Agent 可观测的工件流

对于一种规范的、可检查的 run，将截图、页面源码 dump 和 mock 请求日志写入磁盘：

```bash
bash app/scripts/e2e-agent-review.sh
```

工件落在 `app/test/e2e/artifacts/<timestamp>-agent-review/`。完整详情 + 辅助 API：[`AGENT-OBSERVABILITY.md`](agent-observability.zh-CN.md)。任何失败的测试都会触发 `wdio.conf.ts` 的 `afterTest` hook，将 `failure-*.png` + `failure-*.source.xml` 写入同一运行目录。

---

## Rust 推理提供商 E2E

这些测试（`tests/inference_provider_e2e.rs`）使用 **wiremock** 模拟 HTTP upstream，不需要实时 LLM API 调用。它们覆盖 OpenAI 兼容聊天、Anthropic 认证风格、每模型温度抑制、Ollama 本地提供商和 `/v1` HTTP 端点认证层。

```bash
# 本地：
bash scripts/test-rust-inference-e2e.sh

# 通过 Docker（Linux，与 CI 相同镜像）：
docker compose -f e2e/docker-compose.yml run --rm inference-e2e
```
