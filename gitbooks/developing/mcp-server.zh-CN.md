---
description: 将 OpenHuman Core 作为只读 stdio Model Context Protocol 服务器运行。
icon: plug
lang: zh-CN
---

# MCP 服务器

OpenHuman Core 可以作为可选的 stdio MCP 服务器运行，供 Claude Desktop、Cursor 或 Zed 等本地 MCP 客户端使用。

```bash
openhuman-core mcp
```

该命令不会启动 HTTP JSON-RPC 服务器。它从 stdin 读取换行分隔的 JSON-RPC 2.0 消息，并将 MCP 响应写入 stdout。日志输出到 stderr；添加 `--verbose` 以获得调试输出。

## 客户端来源

在 `initialize` 期间，MCP 服务器捕获 stdio 会话的 `params.clientInfo.name`。名称通过以下方式规范化：修剪首尾空白，转换为小写，将每个非 ASCII 字母数字字符序列替换为单个连字符，然后修剪首尾连字符。例如，`Claude Desktop` 变为 `claude-desktop`，`Cursor` 变为 `cursor`，`Windsurf` 变为 `windsurf`。

如果客户端省略了 `clientInfo.name`、发送空值，或发送一个规范化后结果为空的名称，会话会回退到裸的 `mcp` 来源标签。可写的 MCP 工具应使用此会话来源标签作为记忆来源，以便旧客户端保持现有的 `mcp` 行为，而可识别客户端可以作为 `mcp:<client>` 写入。

## 工具

MCP 表面经过精心设计为只读，并通过现有的控制器注册表以及核心安全策略的读取门禁：

| MCP 工具 | 背后的 RPC | 用途 |
| --- | --- | --- |
| `searxng_search`* | `openhuman.tools_searxng_search` | 搜索配置的自托管 SearXNG 实例。 |
| `memory.search` | `openhuman.memory_tree_search` | 对记忆树块进行关键词搜索。 |
| `memory.recall` | `openhuman.memory_tree_recall` | 对记忆树摘要/块进行语义召回。 |
| `tree.read_chunk` | `openhuman.memory_tree_get_chunk` | 读取搜索或召回返回的一个块。 |
| `tree.browse` | `openhuman.memory_tree_list_chunks` | 分页块列表，支持来源/实体/时间过滤。 |
| `tree.top_entities` | `openhuman.memory_tree_top_entities` | 引用最多的规范化实体，可选按类型过滤。 |
| `tree.list_sources` | `openhuman.memory_tree_list_sources` | 不同的摄入来源及其块计数和最后活动时间戳。 |

* 仅在启用 SearXNG 时存在 `searxng_search`。

`searxng_search` 在启用 SearXNG 时加入 MCP 目录。它接受 `query`、可选的 `categories`（`web`、`news`、`images`）、可选的 `language`，以及可选的 `max_results`（1-50）。
`memory.search` 和 `memory.recall` 接受 `query` 加可选的 `k`（默认 10，上限 50）。`tree.read_chunk` 接受 `chunk_id`。`tree.browse` 接受可选的 `source_kinds`、`source_ids`、`entity_ids`、`since_ms`、`until_ms`、`query`、`k` 和 `offset`。`tree.top_entities` 接受可选的 `kind` 和 `k`。`tree.list_sources` 接受可选的 `user_email_hint`。

在 `config.toml` 或通过环境变量启用 SearXNG：

```toml
[searxng]
enabled = true
base_url = "http://localhost:8080"
max_results = 10
default_language = "en"
timeout_seconds = 10
```

```bash
OPENHUMAN_SEARXNG_ENABLED=true
OPENHUMAN_SEARXNG_BASE_URL=http://localhost:8080
OPENHUMAN_SEARXNG_MAX_RESULTS=10
OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE=en
OPENHUMAN_SEARXNG_TIMEOUT_SECONDS=10
```

## 工具注册表

HTTP JSON-RPC 服务器还暴露一个只读的全局工具注册表，供需要发现元数据而不打开 MCP stdio 会话的智能体和仪表板使用：

| RPC 方法 | 用途 |
| --- | --- |
| `openhuman.tool_registry_list` | 列出 MCP stdio 工具和控制器支持的工具，包含稳定的 `tool_id`、路由、版本、输入/输出 schema、允许的智能体、标签、启用状态和健康状况。 |
| `openhuman.tool_registry_get` | 通过 `tool_id` 返回一个注册表条目，例如 `memory.search` 或 `tools.web_search`。 |

注册表仅用于发现。它不改变工具分派或权限检查；MCP 调用仍通过 `tools/call`，控制器支持的工具仍通过其现有的 JSON-RPC 方法路由。

## 冒烟测试

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | openhuman-core mcp
```

响应应包含来自 `initialize` 的 `capabilities.tools` 和来自 `tools/list` 的精选工具名称。成功的运行向 stdout 写入恰好两行紧凑的 JSON 响应；`notifications/initialized` 消息是通知，没有响应。

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"openhuman-core","version":"<crate version>"},"instructions":"..."}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"memory.search",...},{"name":"memory.recall",...},{"name":"tree.read_chunk",...},{"name":"tree.browse",...},{"name":"tree.top_entities",...},{"name":"tree.list_sources",...}]}}
```
