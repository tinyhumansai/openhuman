---
description: 智能体可直接调用的原生搜索工具——无需 API key。
icon: magnifying-glass
---

# 网络搜索

智能体可以自行搜索实时网页。由服务器端代理（Parallel）支持，所以你无需携带搜索 API key，该工具返回标题、摘要片段和 URL，供后续跟进。

## 适用于

* 研究——"X 的最新动态是什么"。
* 引用追踪——"为我找到 Y 的三个来源"。
* 回答前的事实核查——如果智能体不够自信，会快速搜索。

## 与通用 HTTP 的区别

一个纯粹的 `http_request` 工具可以获取 URL 但无法*找到* URL。网络搜索是发现层：它为智能体挑选正确的 URL，然后交给[网页抓取](web-scraper.zh-CN.md)进行实际阅读。

## 另见

* [网页抓取](web-scraper.zh-CN.md) —— 获取并清理特定 URL。
* [智能 Token 压缩](../token-compression.zh-CN.md) —— 搜索摘要片段在进入模型之前被压缩。
