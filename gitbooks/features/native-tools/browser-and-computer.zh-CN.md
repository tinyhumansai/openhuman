---
description: 原生打开 URL、截图、点击、输入、移动鼠标。
icon: display
---

# 浏览器与计算机控制

当智能体需要像人一样*使用*你的机器时——打开页面、截图、点击按钮、输入短语——这些工具就是它做这些事的方式。

## 浏览器

* **打开**一个 URL，进入智能体可以回读的嵌入式 webview。
* **截图**当前页面。
* **检查**图像输出和元数据，以便智能体描述它看到的内容。

浏览器界面通过 CEF（Chromium Embedded Framework）运行，并包含一个安全层，限制页面能做什么。参见 [Chromium Embedded Framework](../../developing/cef.zh-CN.md) 了解平台详情。

## 计算机（鼠标 + 键盘）

* **鼠标**——移动、点击、拖拽。
* **键盘**——输入文本、发送快捷键。
* **类人路径**——移动和点击遵循类人轨迹，而非瞬移，因此不会触发简单的机器人检测。

## 适用于

* 驱动没有 API 或没有[原生集成](../integrations/README.zh-CN.md)的网站。
* 单次截图不够的多步骤 UI 流程。
* 在聊天中自动化本地应用。

## 另见

* [网页抓取](web-scraper.zh-CN.md) —— 当你只需要文章而非整个页面时。
* [Chromium Embedded Framework](../../developing/cef.zh-CN.md) —— 运行时浏览器层。
