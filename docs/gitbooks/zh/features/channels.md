---
description: >-
  OpenHuman 可双向交互的消息应用：消息如何到达代理、回复和主动消息如何发送，
  以及各个渠道如何登录。
icon: messages-square
---

# 消息渠道

渠道是代理用来与你交流的消息应用。你可以在平时使用的平台上给代理发消息，代理也会在那里回复。渠道与[集成](../../en/features/integrations/README.md)不同：集成主要供代理读取信息，例如邮箱、日历或 CRM；渠道则支持双向对话。

渠道提供方会把平台消息规范化为带类型的信封，并通过配置的提供方发送带类型的回复。OpenHuman 在主机侧负责账户策略、授权、代理执行和持久化对话会话；提供方传输由 TinyChannels v2 模块通过总线契约运行。主机通过带类型的 TinyBus 请求和回调与模块通信，不会链接提供方实现。模块会准备提供方身份和有序消息部分，并拥有规范的带类型渠道与 Web 事件契约；OpenHuman 检查访问权限、选择代理，并在运行轮次前将消息绑定到持久化会话。通用轮次准入和排队机制由 TinyAgents 提供。

## 渠道的工作方式

渠道包含两个方向。

**接收消息。** 提供方模块会把消息、地址、发送者和有序内容规范化为带类型的信封。OpenHuman 主机应用授权和路由策略，然后启动或恢复持久化对话会话。部分平台支持 `/models` 和 `/model` 命令，可为该发送者的会话切换模型。Telegram 还支持远程控制命令。

**发送消息。** 带类型的回复内容由提供方模块发送到原回复目标；平台支持时会保留线程地址。Web 聊天、TUI 和中继的 Web 事件共用 TinyChannels Web 输出模块投影代理进度；网关也可能发送独立的、面向提供方的终止事件。触发器或定时任务启动时，即使没有收到新消息，渠道也可以主动发送消息。只有配置了默认投递目标的渠道才会收到主动消息；其他渠道会被跳过。

平台支持时，渠道可以显示正在输入状态、流式发送草稿、以线程方式回复、发送媒体或语音便笺，以及添加表情回应。这些能力由各提供方决定：模块会报告已配置提供方支持的功能，OpenHuman 不会模拟不支持的操作。

## 支持的渠道

OpenHuman 有 15 种渠道提供方，其中 14 种内置于已发布的桌面应用。WhatsApp Web 受 `whatsapp-web` 构建功能控制，而已发布应用未启用该功能，因此无法在应用中开启。应用内 Web 聊天是内置功能，不属于渠道提供方。另有一个独立的 `cli` 渠道供 `openhuman-core` 终端程序使用。应用中有八种渠道提供设置流程：Telegram、Discord、Web、iMessage、Lark/Feishu、DingTalk、Email 和元宝。其余渠道需要手动在 `config.toml` 中启用。

| 渠道 | 方向 | 接收传输方式 | 凭据 | 设置界面 |
| --- | --- | --- | --- | --- |
| Telegram | 双向 | Bot API 长轮询 | 通过 OpenHuman 托管私聊连接，或使用自己的 BotFather 令牌 | 有 |
| Discord | 双向 | Gateway | 自有机器人令牌、OAuth 安装或托管账户关联 | 有 |
| Web | 双向 | 应用内 | 内置，无需设置（本地） | 有 |
| iMessage | 双向 | macOS 信息（AppleScript） | 仅本地，无凭据（需要“完全磁盘访问”权限） | 有 |
| Lark / Feishu | 双向 | WebSocket 或 webhook | 自有应用 ID 和密钥 | 有 |
| DingTalk | 双向 | Stream Mode WebSocket | 自有客户端 ID 和密钥 | 有 |
| 元宝（Yuanbao） | 双向 | WebSocket | 自有 AppID 和 AppSecret | 有 |
| Slack | 双向 | Events/socket | 自有机器人令牌 | `config.toml` |
| WhatsApp | 双向 | Meta Cloud webhook | 自有访问令牌 | `config.toml` |
| IRC | 双向 | 持久连接 | 自有服务器和昵称 | `config.toml` |
| Signal | 双向 | signal-cli REST 事件 | 自有已关联的 signal-cli 账户 | `config.toml` |
| Mattermost | 双向 | REST API 轮询 | 自有机器人令牌 | `config.toml` |
| QQ | 双向 | WebSocket | 自有机器人凭据 | `config.toml` |
| Linq | 双向（短信） | Webhook | 自有 API 令牌 | `config.toml` |
| Email | 双向 | IMAP IDLE 和 SMTP | 自有邮箱凭据 | 有 |

WhatsApp 还有一个实验性的点对点版本，由 `whatsapp-web` 功能控制。使用 webhook 的渠道通过 HTTP 推送接收消息，因此需要提供方可访问的 HTTPS 端点。

Telegram 支持输入状态指示和实时草稿更新。审批交互取决于提供方声明的能力和配置的渠道界面。Discord 增加了原生线程回复。媒体、语音和地址功能也因提供方和配置能力而异。Web 支持富文本，且完全在本地运行。

### Email

Email 是原生、自托管的连接器，不需要第三方代理。它通过 IMAP IDLE 推送接收邮件，因此新邮件通常在数秒内到达代理。根据 RFC 要求，连接大约每 29 分钟刷新一次。回复通过 SMTP 发送，并支持完整附件；你可以使用自己配置的任意邮件服务商和地址。

`allowed_senders` 列表控制哪些发件人可以联系代理。请设置为你信任的地址。在 `config.toml` 中，空列表表示拒绝所有人。连接设置界面的行为不同：空白字段会变成 `["*"]`，允许所有发件人。如果不希望陌生人向代理发消息，请不要留空。

## 凭据方式

渠道支持三种登录方式。

- **通过 OpenHuman 托管连接。** 由 OpenHuman 后端代理的一键加密连接。适用于 Telegram（直接给托管机器人发消息）和 Discord（关联账户或通过 OAuth 安装）。令牌不会存储在本机。
- **使用自己的凭据。** 你提供机器人令牌、API 密钥和密钥，或应用凭据。Telegram（BotFather 令牌）、Discord、Slack、WhatsApp、Lark/Feishu、DingTalk、元宝、Signal、Mattermost、QQ、Linq、IRC 和 Email 均支持此方式。你拥有最高控制权，同时也负责平台账户、速率限制和 webhook 端点。
- **本地运行，无需凭据。** Web 聊天和 iMessage 不需要令牌。Web 在桌面应用内运行。iMessage 通过 AppleScript 操作本机 macOS 信息应用，因此需要授予“完全磁盘访问”权限。两者都会将消息保留在本机。

任何方式的密钥都会经过 OpenHuman 凭据层处理，并在静态存储时加密（参见[隐私与安全](../../en/features/privacy-and-security.md)）。在界面中管理的渠道不会把明文密钥写入 `config.toml`。

## 连接渠道

请在左侧边栏的**连接 > 渠道**中设置渠道，而不是在“设置”中。打开该标签页，选择平台卡片，然后按照设置卡片操作：

- **Discord。** 选择通过 OpenHuman 托管连接（关联账户或通过 OAuth 安装机器人），也可以粘贴自己的机器人令牌。
- **Telegram。** 给 OpenHuman 托管机器人发消息以关联账户，或粘贴 BotFather 令牌。

Slack 作为应用连接在**连接 > OAuth**中，让代理可以读取和操作 Slack。它不会作为可双向对话的渠道出现在“渠道”标签页中。

## 选择默认渠道

在**连接 > 渠道**中可以选择当前路由。没有指定收件人时，主动消息（例如定时任务和触发器发出的消息）会使用该路由。默认值是应用内 Web 聊天。更改默认渠道会立即生效，无需重启，面板也会显示当前渠道。对收到的消息进行回复时，总是返回原渠道，不受默认设置影响。

## 延伸阅读

- [集成](../../en/features/integrations/README.md)：代理读取上下文的数据源目录。
- [触发器](../../en/features/integrations/triggers.md)：触发主动渠道消息的实时事件。
- [隐私与安全](../../en/features/privacy-and-security.md)：凭据存储位置和后端边界。
- [操作系统密钥环与密钥存储](../../en/features/os-keyring-and-secret-storage.md)：渠道密钥的静态保护。
