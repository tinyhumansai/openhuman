---
description: >-
  可选、自愿开启的本地 AI，通过 Ollama 或 LM Studio 提供。
  为记忆嵌入向量、摘要树构建和后台推理循环提供端侧支持。聊天/视觉/语音走云端。
icon: microchip
---

# 本地 AI（可选）

OpenHuman 可以为以下工作负载在你机器上运行本地模型：当本地保留数据最为重要时：**记忆嵌入向量、摘要树构建和后台推理循环**。它是**自愿开启**的，默认**关闭**。

这是一个刻意的范围界定。之前的设计尝试将聊天、视觉、STT 和 TTS 全部放在 Gemma 3 的设备上，结果是对硬件较敏感的资源占用，与产品其余部分所需的东西冲突。如今，本地最有价值的东西（循环、低延迟、隐私敏感的内存工作）走本地；最有价值于前沿模型的东西（默认聊天、推理、视觉）走云端。

## 开启后什么在本地运行

| 工作负载 | 默认模型 | 实现 |
| ------------------------- | --------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| **记忆嵌入向量** | `all-minilm:latest` | `src/openhuman/embeddings/ollama.rs`——用于[记忆树](../obsidian-wiki/memory-tree.zh-CN.md)向量搜索。 |
| **摘要树构建** | `gemma3:1b-it-qat`（可配置） | `src/openhuman/tree_summarizer/ops.rs`——记忆树的源/主题/全局摘要构建器。 |
| **心跳循环** | 小型聊天模型 | `src/openhuman/heartbeat/`——周期性后台反思。 |
| **学习 / 反思** | 小型聊天模型 | `src/openhuman/learning/reflection.rs`——巩固所学内容的通过。 |
| **潜意识** | 小型聊天模型 | `src/openhuman/subconscious/executor.rs`——后台评估循环。 |

每个都是**按功能开启的 opt-in flag**。开启本地 AI 不会静默将所有内容路由到它，你选择工作负载。

## 什么留在云端

| 工作负载 | 为什么走云端 |
| ------------------ | --------------------------------------------------------------------------------------------------- |
| **聊天（默认）** | 前沿推理质量。通过[模型路由器](README.zh-CN.md)在单一订阅下路由。 |
| **视觉** | 同上。 |
| **STT** | 后端代理转录（`src/openhuman/voice/cloud_transcribe.rs`）。 |
| **TTS** | 底层托管[文字转语音](../native-tools/voice.zh-CN.md)（`reply_speech.rs`）。 |
| **网络搜索** | 后端代理（你的机器上没有 API key）。 |

对于**轻量级或中等聊天 hint**（`hint:reaction`、`hint:classify`、`hint:format`、`hint:sentiment`、`hint:summarize`、`hint:medium`、`hint:tool_lite`），当本地 AI 开启且 Ollama 可达时，[路由器](README.zh-CN.md)会优先使用本地 provider。重型 hint（`hint:reasoning`、`hint:agentic`、`hint:coding`）走云端。

## 工作原理

在底层，OpenHuman 支持两种本地 provider 路径：

* [Ollama](https://ollama.com)，用于捆绑模型生命周期、嵌入向量和现有模型资产流。
* [LM Studio](https://lmstudio.ai)，通过其本地 OpenAI 兼容服务器用于聊天风格本地推理。

对于 Ollama，OpenHuman 在可能的情况下与其 OpenAI 兼容的 `/v1` 端点对话。这意味着：

* `OpenAiCompatibleProvider`（`src/openhuman/providers/compatible.rs`）与 Ollama 的包装方式与与远程 OpenAI 风格 provider 完全相同。没有特殊案例代码路径。
* Provider 路由器在启动时创建一个_健康门控_的本地 provider。如果 Ollama 不可达，请求透明地回退到远程 provider，没有破碎状态。
* 模型按需由 Ollama 拉取并缓存在其自己的存储中。OpenHuman 自己不附带权重。

对于 LM Studio，设置 `local_ai.provider = "lm_studio"` 并确保 LM Studio 本地服务器正在运行。OpenHuman 默认为 `http://localhost:1234/v1`，探测 `GET /v1/models`，并将聊天请求发送到 `POST /v1/chat/completions`。你可以用 `local_ai.base_url`、`OPENHUMAN_LM_STUDIO_BASE_URL` 或 `LM_STUDIO_BASE_URL` 覆盖端点。

## 选择加入

本地 AI 由 core 配置中的两个 flag 门控（`src/openhuman/config/schema/local_ai.rs`）：

| Flag | 默认 | 含义 |
| ------------------------------------ | ------- | ------------------------------------------------------------------- |
| `local_ai.runtime_enabled` | `false` | 主开关。`false` ⇒ 根本不创建本地 provider。 |
| `local_ai.opt_in_confirmed` | `false` | 明确的 opt-in 标记。除非你重新 opt-in，否则 Bootstrap 强制为 `false`。 |
| `local_ai.provider` | `ollama` | 本地 provider：`ollama` 或 `lm_studio`。 |
| `local_ai.base_url` | 未设置 | 可选的 provider URL。LM Studio 默认为 `http://localhost:1234/v1`。 |
| `local_ai.usage.embeddings` | `false` | 使用本地进行记忆嵌入向量。 |
| `local_ai.usage.heartbeat` | `false` | 使用本地进行心跳循环。 |
| `local_ai.usage.learning_reflection` | `false` | 使用本地进行学习通过。 |
| `local_ai.usage.subconscious` | `false` | 使用本地进行潜意识循环。 |

在桌面 app 中，**设置 → AI 与技能 → 本地 AI** 暴露预设，选择一个（"仅嵌入向量"、"记忆 + 反思"、"全部本地"），正确的 flag 组合会为你设置。状态（Ollama 可达性、模型可用性、每个子系统启用）通过 `openhuman.inference_status` 实时暴露。

## 何时开启

如果以下任一为真，开启本地 AI 是值得的：

* 你摄入大量邮件 / 聊天并希望**嵌入向量永不离开机器**。
* 你希望**摘要树构建**离线工作。
* 你对后台反思（"潜意识"）循环隐私敏感。

如果你的连接源很少，云端路径更快，隐私收益很小，则**不值得**开启。也有硬件成本：Ollama 和一个小型 Gemma 模型需要几 GB 的 RAM 并拉取几 GB 的权重。

## 你需要什么

* 安装并运行本地的 [**Ollama**](https://ollama.com)，或启用本地服务器的 [**LM Studio**](https://lmstudio.ai)。
* 模型有足够的磁盘（`gemma3:1b-it-qat` \~700 MB，`all-minilm:latest` \~23 MB）。
* 有足够的 RAM 保持模型驻留（建议 8 GB+，理想 16 GB+）。

OpenHuman 处理其余：生命周期（`src/openhuman/inference/local/service/`）、API 客户端、健康检查，以及当本地 provider 消失时优雅地回退到远程。

### LM Studio 故障排除

* 确认 LM Studio 本地服务器已启用并在 `http://localhost:1234/v1` 可达。
* 在调用 OpenHuman 之前在 LM Studio 中加载所选模型。当配置的 `local_ai.chat_model_id` 不在 `/v1/models` 中时，诊断报告 `load_lm_studio_model`。
* 如果 LM Studio 使用不同端口，设置 `local_ai.base_url` 或 `OPENHUMAN_LM_STUDIO_BASE_URL`。
* LM Studio 模型下载在 LM Studio 内部管理。OpenHuman 不会从本地资产下载控制中拉取 LM Studio 模型。

## 另见

* [记忆树](../obsidian-wiki/memory-tree.zh-CN.md)。本地嵌入向量 + 摘要 powering 什么。
* [自动模型路由](README.zh-CN.md)。轻量聊天 hint 如何优先使用本地 provider。
* [隐私与安全](../privacy-and-security.zh-CN.md)。当你 opt-in 时什么移至端侧。
