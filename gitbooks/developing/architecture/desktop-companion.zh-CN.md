---
description: Desktop Companion 领域 —— Clicky 风格的交互循环，将热键、语音、屏幕智能、LLM、TTS 和视觉指向整合为单一产品体验。
icon: robot
---

# Desktop Companion (`src/openhuman/desktop_companion/`)

Desktop Companion 编排一个 Clicky 风格的交互循环：热键激活、麦克风捕获、屏幕上下文、LLM 推理、语音合成和视觉指向。它复用现有构建块，而非重新实现它们。

## 构建块

| 模块 | 提供的能力 | 路径 |
|--------|-----------------|------|
| **screen_intelligence** | 权限门控的捕获会话、`capture_now()`、`VisionSummary`、`AppContextInfo` | `src/openhuman/screen_intelligence/` |
| **voice** | 热键监听器（push/tap）、音频捕获、云端 STT（Whisper）、TTS (`reply_speech`) | `src/openhuman/voice/` |
| **meet_agent** | LLM 编排模式（STT -> LLM -> TTS）、WAV 打包 | `src/openhuman/meet_agent/` |
| **overlay** | 浮动 UI 表面、注意力事件、打字机气泡 | `src/openhuman/overlay/` |
| **provider_surfaces** | 连接应用事件队列 (`ingest_event`, `list_queue`) | `src/openhuman/provider_surfaces/` |
| **accessibility** | 前台应用上下文 (`foreground_context()`) | `src/openhuman/accessibility/` |

## 模块布局

```text
src/openhuman/desktop_companion/
  mod.rs          — 模块导出（轻量）
  types.rs        — CompanionState enum、CompanionConfig、ConversationTurn、会话 param/result 类型
  session.rs      — 单例会话生命周期、状态机、TTL、对话历史
  pipeline.rs     — STT -> 屏幕上下文 -> LLM -> TTS -> 指向编排
  pointing.rs     — [POINT:x,y:label:screenN] 标签解析器、多显示器坐标映射
  handoff.rs      — 连接应用动作的 provider-surface 队列匹配
  bus.rs          — CompanionStateChangedEvent 的广播通道
  schemas.rs      — RPC 控制器 (companion_start_session, companion_stop_session 等)
```

## 状态机

```text
Idle -> Listening -> Thinking -> Speaking -> Pointing -> Idle
                                    |           |
                                    v           v
                                 Listening   Listening  (中断)

任何状态 -> Error -> Idle (重置)
```

有效转换由 `session::is_valid_transition()` 强制执行。关键路径：

- **Happy path**：Idle -> Listening -> Thinking -> Speaking -> Pointing -> Idle
- **无指向**：Thinking -> Speaking -> Idle（响应中没有 POINT 标签）
- **中断**：Speaking/Pointing -> Listening（用户重新激活热键）
- **取消**：Thinking -> Idle（用户在思考中途取消）
- **错误恢复**：Any -> Error -> Idle

## 交互流水线

`pipeline.rs` 编排单个轮次：

1. **激活** —— 状态转换为 Listening（将由 Tauri 壳层热键桥接驱动，见 PR 2）
2. **STT** —— 通过 `voice::cloud_transcribe`（Whisper）转录音频样本
3. **屏幕上下文** —— `accessibility::foreground_context()` 获取应用名称 + 窗口标题
4. **LLM** —— 通过 `BackendOAuthClient` 进行聊天补全，携带系统提示、屏幕上下文和滚动对话历史（最近 20 轮作为上下文）
5. **解析响应** —— 通过 `pointing::parse_and_map()` 提取 `[POINT:x,y:label:screenN]` 标签
6. **Handoff 检查** —— 扫描响应中的提供商关键词，与 `provider_surfaces` 队列匹配
7. **TTS** —— 通过 `voice::reply_speech`（ElevenLabs）合成语音
8. **指向** —— 为 overlay 动画发射指向目标
9. **返回 Idle**

流水线通过 `CancellationToken` 支持取消 —— Tauri 壳层可以在任何检查点取消（STT、LLM、TTS 阶段之间）。

文本输入也通过 `run_text_turn()` 支持，跳过 STT。

## 会话生命周期

- **一次一个会话** —— 由进程级 `Mutex<Option<CompanionSessionInner>>` 强制执行
- **需要同意** —— `start_session` 拒绝 `consent=false`
- **TTL 强制执行** —— 当 `status()` 检测到 TTL 已过时，会话自动过期
- **对话历史** —— 上限 50 轮，溢出时最旧的被丢弃

## RPC 表面

命名空间：`companion`。所有方法都通过标准控制器注册表。

| 方法 | 说明 |
|--------|-------------|
| `companion_start_session` | 以显式同意 + 可选 TTL 启动会话 |
| `companion_stop_session` | 结束活跃会话 |
| `companion_status` | 当前状态、会话信息、剩余 TTL |
| `companion_config_get` | 读取 companion 配置 |
| `companion_config_set` | 更新 companion 配置 |

## 事件总线

`CompanionStateChangedEvent` 通过 `tokio::sync::broadcast` 通道广播（与 `overlay::bus` 相同模式）。三个 `DomainEvent` 变体路由到 `"companion"` 领域：

- `CompanionSessionStarted { session_id }`
- `CompanionStateChanged { session_id, state, previous_state }`
- `CompanionSessionEnded { session_id, reason }`

## 指向系统

LLM 响应可以嵌入 `[POINT:x,y:label:screenN]` 标签。`pointing.rs`：

- 通过正则解析标签
- 使用 `ScreenGeometry` 将屏幕相对坐标映射为绝对桌面坐标
- 将坐标钳制到屏幕边界
- 索引越界时回退到 screen 0
- 从显示文本中剥离标签

## Provider-surface handoff

`handoff.rs` 扫描清理后的 LLM 响应文本中的提供商关键词（slack、discord、telegram 等），并将它们与 `provider_surfaces` 队列中的条目匹配。当找到匹配时，`HandoffEvent` 被包含在 `TurnResult` 中，供 Tauri 壳层 / overlay 展示。

## 平台范围

- **macOS**：完整支持 —— 热键、屏幕捕获、指向、TTS、overlay
- **Windows/Linux**：部分 —— 热键可用（rdev），屏幕上下文 stub，无指向

平台特定代码通过 `#[cfg(target_os = "macos")]` 门控。

## 测试

| 文件 | 覆盖范围 |
|------|----------|
| `session_tests.rs` | 会话 CRUD、状态机转换、TTL、同意、对话历史 |
| `pipeline_tests.rs` | 轮次编排、取消、输入验证、系统提示 |
| `pointing_tests.rs` | 标签解析、坐标映射、多显示器、边界情况 |
| `handoff.rs` (inline) | 关键词匹配、空队列、提供商覆盖 |
| `schemas.rs` (inline) | 控制器计数、schema 字段验证 |
| `tests/json_rpc_e2e.rs` | 完整 RPC 往返：start -> status -> config -> stop |
