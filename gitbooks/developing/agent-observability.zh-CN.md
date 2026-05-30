---
description: 使 E2E 测试可调试的工件捕获层。日志、跟踪、截图。
icon: eye
---

# E2E 的 Agent 可观测性

本文档描述了使桌面应用可通过现有 WDIO/Appium/tauri-driver harness 被编码智能体（Codex、Claude Code、Cursor）检查的工件捕获层。

它有意保持精简：一个规范的 onboarding + 隐私流程，包含磁盘截图、页面源码 dump 和 mock 后端请求日志。更广泛的计划见仓库根目录的 `AGENT_OBSERVABILITY_PLAN.md`。

## TL;DR

```bash
bash app/scripts/e2e-agent-review.sh
```

工件落在：

```text
app/test/e2e/artifacts/<ISO-timestamp>-agent-review/
  01-welcome.png
  01-welcome.source.xml
  02-post-welcome.png
  02-post-welcome.source.xml
  03-post-onboarding.png
  03-post-onboarding.source.xml
  04-privacy-panel.png
  04-privacy-panel.source.xml
  mock-requests-after-welcome.json
  mock-requests-after-onboarding.json
  mock-requests-after-privacy.json
  failure-<test>.png              # 仅在失败时
  failure-<test>.source.xml       # 仅在失败时
  meta.json                       # 运行元数据 + 检查点索引
```

脚本最后会打印解析后的工件目录。

## 组成部分

| 组件 | 路径 | 作用 |
|-------|------|------|
| 辅助函数 | `app/test/e2e/helpers/artifacts.ts` | 运行目录、`captureCheckpoint`、`captureFailureArtifacts`、`saveMockRequestLog` |
| WDIO hook | `app/test/wdio.conf.ts` (`afterTest`) | 任何失败测试都会 dump 截图 + 源码 |
| 规范 spec | `app/test/e2e/specs/agent-review.spec.ts` | Welcome → onboarding → 隐私面板，带命名检查点 |
| Wrapper 脚本 | `app/scripts/e2e-agent-review.sh` | 构建 + 运行 + 打印工件目录 |
| 稳定选择器 | `OnboardingNextButton`、`Onboarding` 遮罩层 + 跳过按钮、`WelcomeStep`、`PrivacyPanel` 上的 `data-testid` | 智能体可靠的导航锚点 |

## 环境覆盖

| 变量 | 效果 |
|----------|--------|
| `E2E_ARTIFACT_DIR` | 强制指定运行目录（跳过自动时间戳命名） |
| `E2E_ARTIFACT_ROOT` | 自动生成运行目录的父目录（默认：`app/test/e2e/artifacts`） |
| `E2E_ARTIFACT_LABEL` | 自动生成的运行目录名中使用的标签（默认：`run`；wrapper 设为 `agent-review`） |

## 在新 spec 中使用辅助函数

```ts
import {
  captureCheckpoint,
  saveMockRequestLog,
} from '../helpers/artifacts';
import { getRequestLog } from '../mock-server';

await captureCheckpoint('after-connect-click');
saveMockRequestLog('after-connect-click', getRequestLog());
```

`captureCheckpoint` 会对捕获进行编号，使运行目录按时间顺序阅读。
`captureFailureArtifacts` 已接入 `wdio.conf.ts`，在任何失败测试中自动触发，spec 不应直接调用它。

## 有意排除的范围

- 跨每个组件状态的视觉基线 / 图像差异。
- 每次点击都截图（太吵）。
- 实时集成（Gmail、Notion、Telegram）；仅 mock 服务器。
- 新测试框架 / reporter。

仅在证明此循环有效后才扩展到更多流程。
