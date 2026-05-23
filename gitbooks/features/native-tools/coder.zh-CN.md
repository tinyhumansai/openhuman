---
description: 一个完整的工具集，用于处理真实代码库——读、写、编辑、搜索、git、lint、test。
icon: code
---

# 编码器

编码器系列使 OpenHuman 成为可行的编码伙伴，而不是一个*假装*了解代码库的聊天窗口。

## 系列中的工具

| 工具 | 功能 |
| ---------------- | ----------------------------------------------------------------- |
| `file_read` | 读文件（带行号，像 `cat -n`）。 |
| `file_write` | 写一个新文件。 |
| `edit_file` | 定向编辑——严格唯一性检查的匹配替换。 |
| `apply_patch` | 应用统一 diff。 |
| `glob_search` | 按 glob 模式查找文件。 |
| `grep` | 跨树 ripgrep 风格搜索。 |
| `list_files` | 遍历目录树。 |
| `read_diff` | 两个文件或版本之间的 diff。 |
| `git_operations` | Status、diff、log、blame、branch、commit。 |
| `run_linter` | 运行项目的 linter。 |
| `run_tests` | 运行项目的 test 命令。 |
| `csv_export` | 将查询结果导出为 CSV。 |

## 为什么这些是原生的，而非纯 shell

Shell 工具加 `cat`/`sed`/`awk` 技术上可以完成所有这些。原生工具存在是因为：

* 编辑通过唯一性检查，所以智能体不会意外覆盖错误的行。
* 读取返回智能体可以在后续中引用的行号。
* Git 操作将输出解析为结构化数据，而不是让智能体刮擦 porcelain。
* Lint 和 test 运行连接到项目的实际命令，而非通用猜测。

## 工作区作用域

文件系统工具遵守工作区边界——智能体未经明确许可不能在其外部读写。边界与应用的其余部分用于 `OPENHUMAN_WORKSPACE` 的相同。

## 另见

* [系统与工具](system-and-utilities.zh-CN.md) —— `shell`、`node_exec`、`npm_exec` 用于开发循环的其余部分。
* [智能体协作](agent-coordination.zh-CN.md) —— `todo_write`、`spawn_subagent` 用于更大的重构。
