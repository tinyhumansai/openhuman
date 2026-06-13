---
description: >-
  OpenHuman 智能体开箱即用的完整工具集——研究、编码、
  控制你的机器、安排任务、回复你，以及调用 118+ 第三方服务。
icon: toolbox
---

# 原生工具

OpenHuman 的智能体并非空载交付。智能体背后的每个模型在安装瞬间就有一套精选工具可用——无需插件市场、无需接入 API 密钥、无需注册 MCP 服务器。整个工具带都在盒子里。

本页是索引。每个子页面覆盖一个工具族。

## 为什么原生提供这些工具

纯插件模式意味着工具跑在不同进程里，通过 RPC 交互，各自维护认证和打包逻辑。这对于开放式扩展性没问题，但对于每个智能体都需要的**核心**工具（读文件、搜索网页、编辑代码、设提醒、加入会议），以内置方式提供意味着：

* 一致的错误处理。
* 零安装门槛。
* 所有输出自动经过[智能 Token 压缩](../token-compression.zh-CN.md)。
* 可预测的安全边界——文件系统工具遵守工作区作用域，网络工具通过 OpenHuman 代理。

## 工具带

| 类别 | 包含内容 |
| ------ | -------------- |
| [网络搜索](web-search.zh-CN.md) | 无需自带 API key 搜索实时网页。 |
| [网页抓取](web-scraper.zh-CN.md) | 从任意 URL 拉取干净文本——文章、文档、README。 |
| [编码器](coder.zh-CN.md) | 读/写/编辑/补丁文件，glob，grep，git，lint，test。 |
| [浏览器与计算机控制](browser-and-computer.zh-CN.md) | 打开 URL、截图、点击、输入、移动鼠标。 |
| [定时任务与调度](cron.zh-CN.md) | 循环任务、一次性提醒、定时智能体运行。 |
| [语音](voice.zh-CN.md) | 语音转文字输入、文字转语音输出、实时 Google Meet 智能体。 |
| [记忆工具](memory-tools.zh-CN.md) | 在[记忆树](../obsidian-wiki/memory-tree.zh-CN.md)中召回、存储、遗忘和搜索。 |
| [第三方集成](../integrations/README.zh-CN.md) | 智能体视角中的 [118+ 已连接服务](../integrations/README.zh-CN.md)。 |
| [智能体协作](agent-coordination.zh-CN.md) | 生成子智能体、委托给技能、规划、询问用户。 |
| [系统与工具](system-and-utilities.zh-CN.md) | Shell、node、SQL、当前时间、推送通知、LSP。 |

## 另见

* [智能 Token 压缩](../token-compression.zh-CN.md) —— 保持工具输出成本有界的机制。
* [第三方集成](../integrations/README.zh-CN.md) —— 118+ 目录的面向用户介绍和 OAuth 流程。
* [隐私与安全](../privacy-and-security.zh-CN.md) —— 每个工具运行所在的安全边界。
