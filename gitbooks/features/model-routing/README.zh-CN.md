---
description: >-
  一个订阅，多个模型。任务通过 hint 前缀选择模型：
  推理发给强模型，快速路径发给快模型，视觉发给视觉模型。
icon: route
---

# 自动模型路由

智能体的不同部分需要不同的模型。长推理需要前沿模型。快速的"修这个拼写错误"需要又快又便宜的模型。视觉需要视觉模型。OpenHuman 通过内置**路由 provider**处理这一切，所以你永远不需要考虑它。

## 请求如何被路由

任何聊天调用上的 model 参数可以取两种形式：

- **具体模型名**。例如 `anthropic/claude-sonnet-4`。路由到带该精确模型的默认 provider。
- **Hint 前缀**。例如 `hint:reasoning`。在路由表中查找 hint 并解析为 `(provider, model)` 对。

```rust
// src/openhuman/providers/router.rs
fn resolve(&self, model: &str) -> (usize, String) {
    if let Some(hint) = model.strip_prefix("hint:") {
        if let Some((idx, resolved_model)) = self.routes.get(hint) {
            return (*idx, resolved_model.clone());
        }
    }
    (self.default_index, model.to_string())
}
```

路由器包装了多个预创建的 providers（Anthropic、OpenAI、Google、Groq 等），每次请求选择正确的一个。Hint 可以在运行时重新映射而无需重启 core。

## 常见 hint

| Hint | 典型目标 | 使用场景 |
| --- | --- | --- |
| `hint:reasoning` | 强推理模型 | 多步规划、数学、重度代码轮次 |
| `hint:fast` | 快速/便宜模型 | UI 助手、自动补全、小型分类调用 |
| `hint:vision` | 有视觉能力的模型 | 截图、图像附件、OCR |
| `hint:summarize` | 擅长压缩的模型 | 记忆树摘要构建器 |
| `hint:code` | 代码调优的模型 | 原生编码器轮次 |

精确映射可配置；默认值提供每个 provider 的合理路由。

## 一个订阅

路由在单一 OpenHuman 订阅背后发生。你不需要分别为 Anthropic、OpenAI、Google 等持有单独的 API 密钥，后端经纪访问，路由器为每个任务选择正确的一个。这就是 README 中"一个订阅，多个 provider"的承诺，具体化了。

## 覆盖路由

- **全局**。配置 TOML（`src/openhuman/config/schema/types.rs` 中的 `Config` 结构体）可以在启动时提供自定义路由表。
- **每次调用**。传递具体模型名（无 `hint:` 前缀），路由器回退到带该精确模型的默认 provider。
- **对于技能**。技能可以在其 manifest 中固定一个 hint 或模型。

## 为什么这不是简单的"模型切换器"

路由不是 UI 下拉菜单。智能体循环本身根据它要做什么发出 hint。你不选择模型；*任务*选择。这就是"多模型"和"智能路由"的区别。

## 另见

- [智能 Token 压缩](../token-compression.zh-CN.md)。什么使大型推理调用负担得起。
- [原生工具](../native-tools/README.zh-CN.md)。不同的工具调用暗示不同的路由。
- [本地 AI（可选）](local-ai.zh-CN.md)。轻量聊天 hint 可以在设备上运行。
