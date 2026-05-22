---
description: OpenHuman 如何测试其产品 —— Vitest、cargo test、WDIO E2E。每种测试该放哪里。
icon: vial
lang: zh-CN
---

# 测试策略

OpenHuman 如何测试其产品。"我的测试该放哪里？"的权威答案。 companion 文档为 [`TEST-COVERAGE-MATRIX.md`](../../docs/TEST-COVERAGE-MATRIX.md)。

---

## 测试层级

| 层级 | 存放位置 | 测试内容 | 驱动方式 |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| **Rust 单元测试** | 同一 `*.rs` 文件内的 `#[cfg(test)] mod tests`，或同级 `tests.rs`，或域名下的 `tests/` 子目录（例如 `src/openhuman/channels/tests/`） | 纯领域逻辑、schema、RPC handler 形态、内存状态机 | `cargo test` |
| **Rust 集成测试** | 仓库根目录的 `tests/*.rs` | 完整领域接线，含真实 Tokio 运行时、模拟外部服务、JSON-RPC 端到端（`tests/json_rpc_e2e.rs`）、领域 × 领域交互 | `pnpm test:rust`（调用 `bash scripts/test-rust-with-mock.sh`） |
| **Vitest 单元测试** | 与源码共存于 `app/src/**` 下的 `*.test.ts(x)`，或 `app/src/**/__tests__/` 下 | React 组件、hook、store slice、纯工具函数、service 层适配器 | `pnpm test:unit` |
| **WDIO E2E** | `app/test/e2e/specs/*.spec.ts` | 完整桌面流程：UI → Tauri → core sidecar → JSON-RPC；用户可见行为 | Linux CI: `tauri-driver`（端口 4444）。macOS 本地: Appium Mac2（端口 4723）。详见 [E2E 测试](e2e-testing.zh-CN.md)。 |
| **手动冒烟测试** | [`docs/RELEASE-MANUAL-SMOKE.md`](../../docs/RELEASE-MANUAL-SMOKE.md) | 驱动程序无法断言的 OS 级表面：TCC 权限弹窗、Gatekeeper、代码签名、DMG 安装、OS 原生通知 | 发布切割时由人工执行，在发布 PR 中签字确认 |

---

## 决策树 —— 我的测试该放哪里？

```text
变更是否在 JSON-RPC 边界之后（在 src/ 中）？
├─ 是 —— 是否跨领域或与外部服务通信？
│   ├─ 是 → Rust 集成测试 (tests/*.rs)
│   └─ 否  → Rust 单元测试（源码旁）
└─ 否 —— 变更在 app/ 中
    ├─ 是纯函数、hook、slice 或独立组件？
    │   └─ 是 → Vitest 单元测试 (*.test.tsx 与源码共存)
    └─ 是否用户可见 且 跨越 UI ⇄ Tauri ⇄ sidecar ⇄ JSON-RPC？
        ├─ 是 → WDIO E2E (app/test/e2e/specs/*.spec.ts)
        └─ 是否 OS 级（TCC、Gatekeeper、安装、OS 通知）？
            └─ 是 → 手动冒烟清单
```

如果一项变更触及多个层级，在**每个**触及的层级都写测试。不要用一层替代另一层。

---

## 失败路径要求

覆盖矩阵中的每个功能叶子节点，除了 happy path 外，**至少**还要有一个**失败 / 边界**断言。例如：

- 文件写入工具：happy = 写入了字节；failure = 路径限制拒绝。
- OAuth 流程：happy = 签发了 token；edge = 过期刷新 token 恢复。
- 记忆存储：happy = 存储并召回；edge = 遗忘后再召回返回空。

只断言 happy path 的 spec 是不完整的。

---

## Mock 策略

- **单元 / 集成 / E2E 中禁止真实网络。** 使用共享 mock 后端（`scripts/mock-api-core.mjs`、`scripts/mock-api-server.mjs`、`app/test/e2e/mock-server.ts`）。
- 测试用 admin 端点：`GET /__admin/health`、`POST /__admin/reset`、`POST /__admin/behavior`、`GET /__admin/requests`。
- **外部服务**（Telegram、Slack、Gmail、Notion、Ollama、OpenAI 等）在 mock 后端层面被 stub；测试通过 `getRequestLog()` 断言请求形态。
- 唯一可接受的例外是记录在案的发布切割手动冒烟步骤。

---

## 确定性规则

- 禁止 wall-clock 等待，使用 `waitForApp`、`waitForAppReady`、`waitForWebView` 辅助函数，或显式的元素就绪谓词。
- 禁止共享文件系统状态，每个 E2E spec 在隔离的 `OPENHUMAN_WORKSPACE` 中运行（由 `app/scripts/e2e-run-spec.sh` 创建/清理）。
- 禁止顺序依赖的 spec，每个 spec 必须能独立通过。
- 禁止依赖绝对坐标或动画时序。
- 禁止在 tauri-driver 上通过 `browser.keys()` 使用真实键盘，通过 `browser.execute(...)` 合成（参见 `command-palette.spec.ts` 中的模式）。

---

## 现有 harness 提供的能力

- **Mock 后端引导**：`app/test/e2e/mock-server.ts` 中的 `startMockServer` / `stopMockServer`。
- **Auth 捷径**：`helpers/deep-link-helpers.ts` 中的 `triggerAuthDeepLink` / `triggerAuthDeepLinkBypass` 跳过真实 OAuth。
- **元素辅助函数**：`helpers/element-helpers.ts` 中的 `clickNativeButton`、`waitForWebView`、`clickToggle`，在 spec 中使用这些代替原始的 `XCUIElementType*` 选择器。
- **共享流程**：`helpers/shared-flows.ts` 中的 `completeOnboardingIfVisible`、`navigateViaHash`、`navigateToSkills`、`walkOnboarding`。
- **从 spec 调用 Core RPC**：`helpers/core-rpc.ts` 中的 `callOpenhumanRpc`，当 UI 步骤可能脆弱时直接驱动 sidecar。
- **平台守卫**：`helpers/platform.ts` 中的 `isTauriDriver`、`isMac2`、`supportsExecuteScript`。
- **失败时捕获工件**：`captureFailureArtifacts` 从 `wdio.conf.ts` 运行，截图 + DOM dump 输出到 `app/test/e2e/artifacts/`。

---

## 命名与结构规范

- WDIO spec：端到端产品流用 `<feature-area>-flow.spec.ts`；更窄的表面用 `<feature>.spec.ts`。
- Vitest 同位置：优先 `Component.tsx` + `Component.test.tsx` 同级；仅在组合多个相关测试时使用 `__tests__/`。
- Rust 集成测试：文件名用 snake_case 匹配表面，JSON-RPC 驱动流用 `<feature>_e2e.rs`，跨领域用 `<feature>_integration.rs`。
- 每个 `describe` / `mod tests` 块对应一个功能列表 ID 范围，如果映射不明显，在注释中链接矩阵行。

---

## 合并前门禁

开 PR 前运行。CI 会跑同一套，但本地更快：

```bash
# Rust 核心
cargo fmt --check
cargo check --manifest-path Cargo.toml
cargo clippy --manifest-path Cargo.toml -- -D warnings
cargo test --manifest-path Cargo.toml

# Tauri 壳层
cargo check --manifest-path app/src-tauri/Cargo.toml

# 前端
pnpm typecheck
pnpm lint
pnpm format:check
pnpm test:unit

# 带 mock 后端的 Rust 集成测试
pnpm test:rust

# E2E（慢 —— 仅在行为用户可见变更时运行）
pnpm test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/<your-spec>.spec.ts <id>
```

---

## 无法被驱动程序自动化的 —— 需要手动冒烟

某些表面无法被 WDIO / Appium 驱动，因为它们跨越 OS 级信任边界或硬件路径。完整的清单 + 签字块位于 [`docs/RELEASE-MANUAL-SMOKE.md`](../../docs/RELEASE-MANUAL-SMOKE.md)，该文件是每次发布必须验证内容的权威来源。涵盖示例：

- macOS TCC 权限弹窗（辅助功能、输入监控、屏幕录制、麦克风）
- Gatekeeper 首次启动签名验证
- 代码签名完整性（`codesign --verify --deep --strict`）
- DMG 安装 / 拖入 Applications 流程
- 自动更新下载 + 重启
- Linux OS 原生通知 toast（无显示服务器的 driver 无法看见 Xvfb 之外的 Linux）

如果一项功能没有自动化覆盖，也不在手动冒烟清单上，视为未测试，开一个覆盖缺口。

---

## 覆盖矩阵即契约

[覆盖矩阵](../../docs/TEST-COVERAGE-MATRIX.md) 中的每个功能叶子节点映射到：

1. 一个或多个测试路径，**或**
2. 一个合理的 `🚫` 并附手动冒烟条目。

当你添加 / 删除 / 重命名功能时，**在同一 PR 中更新矩阵行**。CI 将在 #965 落地后守卫此契约。

---

## 不确定时

- 尽可能把测试推到层级栈的**底层**（Rust 单元 > Rust 集成 > Vitest > WDIO）。更低层级更快、更确定、运行成本更低。
- WDIO 用于真正跨越 UI ⇄ Tauri ⇄ sidecar ⇄ JSON-RPC 的行为。不要仅仅因为 UI 存在就通过 WDIO 驱动一个可单元测试的关注点。
- 失败的 happy path 是回归。缺失的失败路径测试是缺口。两者都是 bug。
