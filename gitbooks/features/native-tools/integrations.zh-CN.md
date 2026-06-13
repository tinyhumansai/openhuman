---
description: 智能体对 118+ 已连接第三方服务的视图。
icon: plug
---

# 第三方集成

OpenHuman 的智能体可以通过单一代理工具接口调用 [118+ 第三方服务](../integrations/README.zh-CN.md)——Gmail、Notion、GitHub、Slack、Stripe、日历，以及长长的尾部的服务。

## 它在智能体看来如何

一旦你通过 OAuth 连接了服务，其操作就变为可调用工具。智能体不需要知道工具是与 Gmail 还是与本地文件对话——它只调用工具，代理用你的 token 通过 OpenHuman 后端路由请求，结果像任何其他工具输出一样返回。

一些变为可用的例子：

* "在 Slack 上向 #engineering 发送消息。"
* "在 openhuman 仓库中创建一个 issue。"
* "我日历上明天有什么？"
* "拉取过去 20 笔超过 $1000 的 Stripe charge。"

## 原生 vs 代理

部分服务有**原生 provider**——Rust 模块知道如何直接将服务摄入[记忆树](../obsidian-wiki/memory-tree.zh-CN.md)（例如 Gmail 的原生摄入路径）。其他仅暴露为**代理工具**：智能体可以调用，但没有自动摄入。新的原生 provider 随着功能落地陆续添加。

## 隐私边界

OpenHuman core 从不直接调用任何第三方 API。所有请求都通过 OpenHuman 后端，该后端处理 OAuth token 和速率限制。你的 token 永不以明文形式存储在你机器的磁盘上，智能体只看到工具调用的*结果*，而不是凭据。

## 另见

* [第三方集成（目录）](../integrations/README.zh-CN.md)——面向用户的介绍、OAuth 流程和连接管理。
* [自动拉取](../obsidian-wiki/auto-fetch.zh-CN.md)——已连接服务如何流入记忆树。
* [隐私与安全](../privacy-and-security.zh-CN.md)——完整边界。