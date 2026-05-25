---
description: >-
  118+ 第三方集成——Gmail、Notion、GitHub、Slack、Stripe、日历等，
  一键 OAuth 连接，无需 API 密钥。
icon: plug
---

# 第三方集成（118+）

OpenHuman 搭载对 **118+ 第三方服务**的后端代理访问。任意服务通过托管路径连接都只需在应用内一键 OAuth，无需手动接入 API 密钥，也无需穿梭于插件市场。

底层连接器层由 [Composio](https://composio.dev) 驱动。默认托管模式下，OpenHuman 后端拥有 Composio API 密钥、OAuth token 经纪、速率限制和触发器 webhook 分发。如果你切换到直连模式，core 用你自己的 Composio API 密钥与 Composio 通信；同步工具调用可以工作，但实时触发器 webhook 必须配置在你自己的 webhook 基础设施上。

服务连接后，会同时出现在四个位置：

1. 作为**智能体工具**，模型可以直接调用。
2. 作为**记忆源**，[自动拉取](../obsidian-wiki/auto-fetch.zh-CN.md)每二十分钟将其同步到[记忆树](../obsidian-wiki/memory-tree.zh-CN.md)。
3. 作为**个人化信号**，你在各服务上的活动为你的偏好模型提供数据。
4. 作为**触发器源**，实时事件（新邮件、新 charge、入站 DM）流入[触发器](triggers.zh-CN.md)流水线，可以自动触发智能体操作。

## 目录中的部分服务

目录涵盖生产力、商业、社交、消息和 Google 类目。不完全示例：

| 类别 | 示例 |
| ----------------------- | ---------------------------------------------------- |
| **邮件与日历** | Gmail、Outlook、Google Calendar、Apple Calendar |
| **文档与存储** | Google Docs、Google Drive、Notion、Dropbox、Airtable |
| **代码与开发** | GitHub、Linear、Jira、Figma |
| **通讯** | Slack、Discord、Microsoft Teams、Telegram、WhatsApp |
| **CRM 与销售** | Salesforce、HubSpot |
| **商业与支付** | Stripe、Shopify |
| **项目管理** | Asana、Trello |
| **社交** | Twitter / X、Spotify、YouTube |

## 原生 vs 代理

部分服务有**原生 provider**。Rust 模块知道如何直接将服务摄入记忆树（例如 Gmail 的原生摄入路径）。其他仅暴露为**代理工具**：智能体可以调用，但没有自动摄入。新的原生 provider 随着功能落地陆续添加。

## 连接如何工作

点击任意集成的**连接**。浏览器窗口打开进行 OAuth。登录后，连接变为活跃状态，OpenHuman 在下一个 20 分钟 tick 开始同步。

每个集成显示其当前状态：

* **未连接**。集成尚未设置。
* **已连接**。集成活跃并正在同步。
* **管理**。活跃集成，可重新配置或断开。

你可以随时从 Skills 标签页撤销任何连接。

## 消息渠道

三个集成是特殊的。OpenHuman 用它们*回复*你，而不只是读取：

* **Telegram**。主要消息渠道。双向：发送和接收消息、管理聊天、搜索历史、创建群组、代表你执行 80+ 操作。所有操作通过你自己加密的凭据运行。
* **Discord**。通过 Discord 发送和接收消息。连接你的账户以接收 OpenHuman 消息。
* **Web**。桌面应用内的浏览器聊天界面。消息完全保留在本地。

在**设置 → 自动化与渠道 → 消息渠道**中设置你的默认值。活跃路由状态显示当前使用的渠道。Telegram 提供两种凭据模式：通过 OpenHuman 连接（一键，加密）或提供你自己的凭据以获得最大控制权。

## 技能

除了第三方服务，OpenHuman 还有**技能**——运行在应用内的小型沙盒模块，获取外部数据、按计划运行、转换信息、响应事件。每个技能都强制执行资源限制。技能从 Skills 标签页安装，与其他所有内容一样集成到同一个记忆树。

## 原生语音和工具

有两个功能作为原生功能搭载，而非集成，因为它们对桌面体验是基础性的：

* [**语音**](../native-tools/voice.zh-CN.md)。语音转文字输入、文字转语音输出，加上实时 Google Meet 智能体——加入会议、转录到记忆树、在通话中说话。
* [**原生工具**](../native-tools/README.zh-CN.md)。内置网络搜索、网络抓取，以及完整的文件系统/git/lint/test/grep 编码工具集，智能体开箱即用。

## 隐私边界

OpenHuman core 从不直接调用任何第三方 API。所有请求都通过 OpenHuman 后端，该后端处理 OAuth token 和速率限制。你的 token 永不以明文形式存储在电脑磁盘上，智能体只看到工具调用的*结果*，而不是凭据。

如果你选择直连 Composio 模式，该边界会改变：你本地的 core 使用你自己的 Composio API 密钥，你负责 Composio 账户、速率限制、计费关系，以及触发器投递所需的任何 webhook 端点。

完整边界见[隐私与安全](../privacy-and-security.zh-CN.md)。

## 另见

* [触发器](triggers.zh-CN.md)，已连接集成的实时事件以及它们如何触发智能体操作。
* [从集成自动拉取](../obsidian-wiki/auto-fetch.zh-CN.md)
* [记忆树](../obsidian-wiki/memory-tree.zh-CN.md)
