---
description: >-
  每个记忆块也作为 Markdown 文件存在于与你 Obsidian 兼容的存储库中，
  你可以打开和编辑。灵感来自 Karpathy 的 obsidian-wiki 工作流。
icon: book-open
---

# Obsidian 风格的记忆

<figure><img src="../../.gitbook/assets/image (1).png" alt=""><figcaption><p>OpenHuman 记忆在 Obsidian 中的预览。来自各种来源（GMail、Slack、Whatsapp 等）的数据被组织成一棵记忆树。</p></figcaption></figure>

OpenHuman 的记忆不是一个黑箱。智能体在其上推理的相同块作为普通的 `.md` 文件写入你工作区内的存储库中。你可以在 [Obsidian](https://obsidian.md) 中打开它，浏览、编辑、手动链接笔记，智能体都会看到你的改动。

设计直接灵感来自 [Andrej Karpathy 的 obsidian-wiki 工作流](https://x.com/karpathy/status/2039805659525644595)：一个个人 wiki，你生活中每个有趣的事物最终都成为一个可链接的笔记。

## 存储库在哪里

```text
<workspace>/
└── wiki/
 ├── summaries/ # 自动生成的源 / 主题 / 全局摘要
 ├── notes/ # 你的手写笔记（自由格式）
 └── … # 每个已连接工具包的文件夹
```

`summaries/` 文件夹按层级布局：全局树按日期，源树按源，主题树按实体。每个文件的前置元数据携带来源（源 id、时间范围、作用域），以便智能体可以将任何声明追溯到产生它的块。

## 打开存储库

在桌面 app 中，**记忆**标签页有一个**"在 Obsidian 中查看存储库"**按钮。它使用 `obsidian://open?path=...` 深度链接，所以你需要已安装 Obsidian。

你也可以在任何编辑器中打开该文件夹，它其实就是 Markdown。文件之间的链接使用标准的 `[[wiki-link]]` 语法，因此 Obsidian 的图谱视图、反向链接和标签浏览器开箱即用。

## 手动编辑笔记

`wiki/notes/` 中的任何内容都会被纳入摄取范围。处理 Gmail 和 Slack 的相同流水线会获取你的手写笔记，对它们进行分块、评分，并与其他所有内容一起折叠到主题树和全局树中。

这意味着你可以：

* 将会议笔记放入 `wiki/notes/2026-05-08-board-call.md`，智能体明天就会知道背景。
* 按项目、人物、股票代码维护一个文件，主题树将你的手动笔记视为另一个数据源。
* 批量导入现有 Obsidian 存储库：将 `.md` 文件放入并触发摄入。

## 为什么这很重要

你无法信任你无法读取的记忆。大多数"AI 记忆"系统将状态隐藏在不透明的嵌入中；OpenHuman 的存储库则相反，智能体的记忆**确确实实**就是一个你拥有的 Markdown 文件夹。如果智能体弄错了什么，你可以找到文件，修复它，下一次检索就是正确的。

这也是最干净的导出方式：即使明天不再使用 OpenHuman，你仍然保留一个完整的个人 wiki。

## 另见

* [记忆树](memory-tree.zh-CN.md)。产生存储库的流水线。
* [从集成自动拉取](auto-fetch.zh-CN.md)。存储库如何自行增长。
