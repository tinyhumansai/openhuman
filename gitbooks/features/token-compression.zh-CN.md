---
description: >-
  TokenJuice - 一层规则叠加，在工具输出进入 LLM 上下文之前将其压缩。
  处理成千上万封邮件依然成本低廉。
icon: file-zipper
---

# 智能 Token 压缩

LLM Token 价格不菲，而冗长的工具输出是消耗大多数 Token 的地方。繁忙仓库里的 `git status`、一次 `cargo build` 日志、一个 600 条消息的邮件串，或者针对真实集群的 `docker ps -a`，这些都可能把上下文窗口撑得很大，却几乎不带多少有效信息。

OpenHuman 搭载 **TokenJuice**，这是 [vincentkoc/tokenjuice](https://github.com/vincentkoc/tokenjuice) 的移植版本，直接集成到工具执行路径中。在任何工具结果到达模型之前，TokenJuice 会将输出通过一层规则叠加进行处理，去除噪音、保留信号。

## 三层规则叠加

规则是 JSON，按以下顺序合并，后面的层级覆盖前面的：

<table><thead><tr><th width="134.41796875">层级</th><th>路径</th><th>用途</th></tr></thead><tbody><tr><td><strong>内置</strong></td><td>随二进制文件发布</td><td>为 git、npm、cargo、docker、kubectl、ls 等提供的合理默认值</td></tr><tr><td><strong>用户</strong></td><td><code>~/.config/tokenjuice/rules/</code></td><td>你的个人覆盖，应用于所有项目</td></tr><tr><td><strong>项目</strong></td><td><code>.tokenjuice/rules/</code></td><td>仓库特定的覆盖，纳入版本控制，与团队共享</td></tr></tbody></table>

每条规则命名一个工具/命令模式和一个压缩策略（截断、行去重、折叠空白、删除匹配的正则表达式、摘要分段等）。新规则就是 JSON 文件，无需重新编译。

## 为什么这和记忆有关

TokenJuice 是使[自动拉取](obsidian-wiki/auto-fetch.zh-CN.md)在经济上可行的原因。当 Gmail provider 同步一页 200 条消息时，TokenJuice 在每个规范化的邮件进入构建摘要的模型**之前**就将其压缩。GitHub diff、Slack 频道转储以及其他任何高流量来源同理。

具体来说：通过前沿模型摄入你最近六个月的邮件费用从数百美元降到个位数美元。

## 它在流水线中的位置

```text
工具调用结果
      │
      ▼
TokenJuice（分类 → 匹配规则 → 压缩）
      │
      ▼
LLM 上下文
```

实现：`src/openhuman/tokenjuice/`（`classify.rs`、`reduce.rs`、`rules/compiler.rs`、`tool_integration.rs`）。

## 检查和覆盖

* 在 `~/.config/tokenjuice/rules/` 中放入一个 JSON 文件来全局添加或覆写规则。
* 在仓库内的 `.tokenjuice/rules/` 中放入一个来做同样的项目级设置。
* 使用 `RUST_LOG=openhuman_core::openhuman::tokenjuice=debug` 启动 core，可以查看匹配了什么以及多少输出被裁剪了。

## 另见

* [原生工具](native-tools/README.zh-CN.md)。大多数重型工具输出都经过 TokenJuice。
* [记忆树](obsidian-wiki/memory-tree.zh-CN.md)。压缩输出的下游消费者。
