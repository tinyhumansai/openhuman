---
description: >-
  安装 OpenHuman，完成应用内入门引导（登录、连接 Gmail、
  选择 AI 运行方式），然后对你的记忆树发出第一个请求。
icon: play
---

# 快速入门

本文将引导你完成安装 OpenHuman、完成应用内入门引导，以及发出第一个请求。

OpenHuman 遵循 GNU GPL3 开源许可证，代码库位于 [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)。

***

## 系统要求

OpenHuman 支持 **macOS、Windows 和 Linux** 桌面端。建议 4 GB 以上内存；如果要摄入超大型邮箱或仓库，或在同一台机器上运行[本地模型](../features/model-routing/local-ai.zh-CN.md)，建议 16 GB 以上。

### 权限

首次启动 OpenHuman 时，操作系统会提示授予应用所需的权限（macOS 上的 Accessibility、语音热键的 Input Monitoring，以及计划使用[会议智能体](../features/mascot/meeting-agents.zh-CN.md)时的相机/麦克风）。你随时可以在 **设置 → 自动化与渠道** 中查看和调整这些权限。

***

## 1. 下载并安装

从 [https://tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) 或通过你平台的软件包管理器获取 OpenHuman 桌面应用。安装后打开应用。

## 2. 登录

第一个屏幕是**"登录！让我们开始吧"**。提供多种登录方式，包括社交登录。如果你要将应用指向自定义 core RPC URL（自建后端的情况），还有一个**高级**面板；大多数用户可以忽略它。

{% hint style="info" %}
**无永久锁定。** 登录不会授予 OpenHuman 对任何内容的持续访问权。所有第三方访问都需要在以下步骤中每个集成单独进行明确的 OAuth 批准。
{% endhint %}

## 3. 发出你的第一个请求

一旦 Gmail 完成摄入（首次自动拉取会在二十分钟内触发），可以尝试以下提示：

**简报**

* "过去 12 小时我需要了解什么？"
* "有什么在等着我？"

**跨源查询**

* "总结我今天错过了什么。"
* "这周有哪些关键决策？"
* "从我最近的对话中提取行动项。"
* "Sarah 在邮件和聊天中对这个项目说了什么？"

OpenHuman 自动为每个任务选择合适的模型。参见[自动模型路由](../features/model-routing/)。

***

## 4. 打开 Obsidian 存储库

"记忆"标签页有一个**"在 Obsidian 中查看存储库"**按钮。点击它可以在 [Obsidian](https://obsidian.md) 中打开 `<workspace>/wiki/`。你可以浏览智能体的摘要、放入你自己的笔记，甚至构建手动链接——智能体会在下一次摄入时获取你的编辑。参见 [Obsidian 风格的记忆](../features/obsidian-wiki/)。

***

## 5. 让吉祥物做更多

现在智能体有了记忆和一个模型，产品的其余部分就是给它更多发挥空间：

* [**会议智能体**](../features/mascot/meeting-agents.zh-CN.md) —— 放入一个 Google Meet 链接，吉祥物作为真实参与者加入：它倾听、将笔记记入记忆树、在通话中说话，并实时使用工具。
* [**从集成自动拉取**](../features/obsidian-wiki/auto-fetch.zh-CN.md) —— 从**设置**中连接更多源；每二十分钟调度器将新数据拉入你的树。
* [**原生语音**](../features/native-tools/voice.zh-CN.md) —— 按键说话输入和 TTS 回复，这样你可以和 OpenHuman 对话而不是打字。
* [**潜意识循环**](../features/subconscious.zh-CN.md) —— 让你离开时吉祥物继续处理待办任务。

## 加入社区

OpenHuman 处于早期测试阶段。在这个阶段，反馈和贡献能带来真正的改变。

* **GitHub：** [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)
* **Discord：** [discord.tinyhumans.ai](https://discord.tinyhumans.ai)
