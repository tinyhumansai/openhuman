---
description: >-
  运行 OpenHuman 诊断，选择检查级别，并根据报告修复配置和运行问题。
icon: activity
---

# 使用 doctor 诊断 OpenHuman

doctor 命令会检查所选 OpenHuman 配置和运行环境，并返回稳定的检查 ID、
检查结果和修复提示。它不会自动修复设置，也不会切换当前配置档案。

## 运行报告

在安装了 `openhuman-core` 的终端中运行：

```bash
openhuman-core doctor
```

默认的 `connect` 级别会检查本地配置和工作区，然后对已配置的服务执行有时限、
只读的检查。使用 `--live` 可选择加入实时检查：

```bash
openhuman-core doctor --live
```

CLI 默认使用 `connect`。CLI 的 `--live` 会启用实时检查；JSON-RPC 和 agent 工具
还可以选择 `static` 级别。

使用 `--only` 和 `--skip` 按精确检查 ID 选择项目。每个选项都可以重复使用：

```bash
openhuman-core doctor --only config/parse --only workspace/writable
openhuman-core doctor --skip embeddings/configured-model
```

未知 ID 会在报告中显示为 `doctor/selection` 错误。同一请求中有效的 ID 仍会运行。
完整清单和修复提示见下文。

## 检查级别与费用

| 级别      | 检查内容                                                                               |
| --------- | -------------------------------------------------------------------------------------- |
| `static`  | 本地配置和文件系统检查。部分检查会短暂创建并删除有大小限制的临时文件，以确认访问权限。 |
| `connect` | `static` 检查，以及对已配置本地服务和网络端点进行有时限、只读检查。这是默认级别。      |
| `live`    | `connect` 检查，以及一次性推理和嵌入请求、一次临时记忆存储/读取/删除测试。             |

{% hint style="warning" %}
实时检查可能消耗推理或记忆服务额度。只有在你准备好发出这些请求时才使用
`--live`。记忆往返测试会存入一个隔离的临时条目，并在完成后尝试删除；如果无法
确认已清理，报告会说明这一点。本地 SQLite 记忆引擎访问数据库时，可能会更新
派生的 `-shm` 或空 `-wal` 辅助文件状态。
{% endhint %}

每个推理探测都会发送简短的 `Reply OK` 提示，并要求提供方在请求层将输出限制为最多
5 个 token。解析到相同端点、凭据和实际模型的角色会共用一次补全请求，但仍各自生成
检查结果。如果路由无法在请求层强制输出上限，doctor 会跳过该路由且不会调用模型。
OpenAI Codex OAuth 使用的 Responses API 请求没有输出 token 上限字段，因此对应的
`inference/live/<role>` 会跳过；这不表示 Codex 已通过健康检查。

agent 工具也遵循此规则：日常诊断可要求助手运行 `doctor_health`。只有当你明确
要求执行可能计费的实时探测时，助手才应选择 live 级别。

## 输出模式和退出码

命令支持人类可读表格、JSON 输出和 lint 模式：

```bash
openhuman-core doctor --json
openhuman-core doctor --lint
openhuman-core doctor --lint --severity-min warn --json
```

| 模式            | 结果                                                                                   |
| --------------- | -------------------------------------------------------------------------------------- |
| 默认模式        | 按检查类别分组输出表格和摘要。成功生成报告后返回 `0`。                                 |
| `--json`        | 输出 v2 JSON 报告。成功生成报告后始终返回 `0`，不受检查结果影响。                      |
| `--lint`        | 没有达到最低严重级别的结果时返回 `0`；有一个或多个时返回 `1`；无法生成报告时返回 `2`。 |
| `--lint --json` | 输出 JSON，同时保留 lint 的 `0`、`1` 或 `2` 退出码规则。                               |

`--severity-min` 可设为 `info`、`warn` 或 `error`，默认是 `error`。`pass` 和
`skipped` 不会达到阈值。若希望 CI 对警告和错误都失败：

```bash
openhuman-core doctor --lint --severity-min warn
```

## 选择配置档案

在桌面端，`--profile <id>` 会检查指定配置档案，而不会激活它或切换当前用户：

```bash
openhuman-core doctor --profile work
```

配置档案以只读方式加载。档案不存在或布局不受支持时，报告会给出对应结果和修复
提示。SaaS 主机始终使用当前租户的强制配置；配置档案选项不能用于选择其他租户。

## 阅读报告

报告使用 schema 版本 `2`。其中包括已知的配置档案和配置来源、请求的级别、已运行
或跳过的检查 ID、发现项、严重程度摘要以及 `ok`。只有当某项严重程度为 `error`
时，`ok` 才为 false；警告仍是提示。在 lint 模式中，退出码由单独的严重程度阈值决定。

每项发现都有简短说明和 `fix_hint`。报告可以注明配置路径，以及值来自命令行、环境
变量、`.env`、配置文件或默认值。报告不会包含凭据或解析后的配置值。`config/source`
可能会列出加载的配置文件和 `.env` 路径，帮助你查明设置来源。

doctor 没有 `--fix` 模式。请自行应用修复，然后重新运行相关检查或完整报告。

## 检查清单

以下是已注册的检查 ID。动态 ID 的最后一段会包含配置中的模块、服务器、提供方或
推理角色。报告会针对所检查的具体设置或服务提供更具体的提示。

| 检查 ID                              | 级别    | 检查内容                                                                                                                                      | 出现问题时的下一步                                                                                         |
| ------------------------------------ | ------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `config/parse`                       | static  | 配置解析，以及无效文件或备份的恢复状态。                                                                                                      | 修复 `config.toml`；如发生恢复，请检查并还原有效备份。                                                     |
| `config/unknown-key`                 | static  | 因不受支持而被忽略的配置键。                                                                                                                  | 按建议改为受支持的字段名。                                                                                 |
| `config/legacy-key`                  | static  | 加载时迁移的旧配置键。                                                                                                                        | 将配置改为当前键名。                                                                                       |
| `config/env-override`                | static  | 环境变量或 `.env` 中覆盖文件值的设置。                                                                                                        | 选择一个来源，并移除被覆盖的设置。                                                                         |
| `config/source`                      | static  | 配置和 `.env` 路径及值来源。                                                                                                                  | 根据报告的来源排查优先级。                                                                                 |
| `inference/provider-ref`             | static  | 引用了不可用推理提供方的设置。                                                                                                                | 选择已注册的提供方，并修正报告的配置字段。                                                                 |
| `inference/construct`                | static  | 配置的模型路由是否能使用所需凭据创建。                                                                                                        | 修正提供方设置并配置所需凭据。                                                                             |
| `inference/temperature`              | static  | 模型路由上的温度设置是否受支持。                                                                                                              | 为报告中的路由设置受支持的温度。                                                                           |
| `embeddings/config`                  | static  | 嵌入提供方、模型、路由和向量维度。                                                                                                            | 配置相互匹配的提供方、模型和维度。                                                                         |
| `components/referenced-but-disabled` | static  | 配置引用了已禁用的组件。                                                                                                                      | 启用组件或移除依赖它的配置。                                                                               |
| `config/runtime-settings`            | static  | 被忽略的回退提供方，以及不完整的模型、委托代理或已启用频道设置。                                                                              | 按提示完成、移除或启用对应字段。                                                                           |
| `modules/source-fallback`            | static  | 模块来源是否从预期来源回退。                                                                                                                  | 明确配置预期使用的模块来源。                                                                               |
| `modules/override-invalid`           | static  | 无效的模块覆盖设置。                                                                                                                          | 修正或移除报告的覆盖项。                                                                                   |
| `storage/url`                        | static  | 配置的存储 URL 是否使用此构建支持的驱动。                                                                                                     | 使用受支持的 URL 方案并启用对应驱动。                                                                      |
| `workspace/action-dir`               | static  | agent 操作目录是否可用。                                                                                                                      | 创建该目录或授予进程所需访问权限。                                                                         |
| `workspace/writable`                 | static  | 所选工作区是否允许受限写入测试。                                                                                                              | 授予工作区写入权限。                                                                                       |
| `workspace/layout`                   | static  | 工作区必需文件和可用磁盘空间。                                                                                                                | 创建报告中缺失的项目或释放磁盘空间。                                                                       |
| `runtime/environment`                | static  | `HOME`、`SHELL`、`git` 和 `curl` 等本地环境。                                                                                                 | 安装或配置报告中指出的本地依赖。                                                                           |
| `runtime/agent-sdk`                  | connect | 已配置的 Claude Agent SDK 命令及版本。                                                                                                        | 安装或配置 SDK 命令，然后重新运行 doctor。                                                                 |
| `secrets/master-key-env`             | static  | 是否同时设置了两个主密钥环境变量。                                                                                                            | 只保留预期的主密钥来源。                                                                                   |
| `network/proxy`                      | static  | 已配置的网络代理是否有效。                                                                                                                    | 修正检查指出的代理设置。                                                                                   |
| `memory/root`                        | static  | 已配置的记忆根目录是否有效。                                                                                                                  | 修正报告中的记忆根目录路径。                                                                               |
| `profile/exists`                     | static  | 桌面配置档案是否存在，布局是否受支持且配置有效。                                                                                              | 选择具有受支持布局和有效配置的现有档案。                                                                   |
| `profile/provisioned`                | connect | 所选租户档案是否已配置并可用。                                                                                                                | 先配置租户档案，再重新运行 doctor。                                                                        |
| `memory/binding`                     | static  | 所选记忆引擎及其配置。                                                                                                                        | 选择可用的记忆驱动。                                                                                       |
| `memory/health`                      | connect | 所选记忆引擎是否健康。                                                                                                                        | 恢复记忆驱动的可用性并重新运行检查。                                                                       |
| `memory/roundtrip`                   | live    | 临时记忆条目能否存储、读取和删除。                                                                                                            | 检查所选引擎；若未确认清理，请查看隔离的 doctor 服务范围。                                                 |
| `storage/connect`                    | connect | 对配置存储驱动的只读访问。不会创建缺失的 SQLite 数据库；现有数据库及 WAL 数据保持不变，但 SQLite 可能创建派生的 `-shm` 或空 `-wal` 辅助文件。 | 检查存储是否存在且可读；远程存储不可用时检查连接。                                                         |
| `storage/layout`                     | static  | 是否有未完成的配置档案存储布局迁移。                                                                                                          | 停止 OpenHuman，通过 Settings 或 CLI 继续迁移。                                                            |
| `secrets/keyring`                    | connect | 是否能安全检查已配置的密钥环。非交互 macOS 检查可能跳过。                                                                                     | 在交互环境运行，或配置可用的密钥环后端。                                                                   |
| `embeddings/configured-model`        | connect | 配置的 Ollama 嵌入模型是否可用。其他提供方会被此项跳过。                                                                                      | 启动配置的 Ollama 服务，或修正 URL 和模型。                                                                |
| `modules/<id>`                       | connect | 每个已配置模块的状态。禁用的模块会跳过。                                                                                                      | 修复或启用模块，并检查其配置摘要。                                                                         |
| `mcp/<server>`                       | connect | 每个已配置 MCP 服务器的连接状态。                                                                                                             | 启动服务器或修正连接设置。                                                                                 |
| `skills/discovery`                   | static  | 已安装的 skill 清单是否可读。                                                                                                                 | 修复报告路径上的清单文件。                                                                                 |
| `skills/github-preflight`            | connect | skill 所需的 GitHub 访问。                                                                                                                    | 解决报告中的 GitHub 访问或仓库问题。                                                                       |
| `sandbox/backend`                    | connect | 配置的 Docker 沙箱后端是否可用。                                                                                                              | 安装或启动 Docker，或选择可用的沙箱后端。                                                                  |
| `channels/<provider>`                | connect | 每个已配置消息提供方的健康状态。                                                                                                              | 恢复提供方连接并重新运行检查。                                                                             |
| `integrations/composio`              | connect | 所选配置档案中的 Composio 是否可用。                                                                                                          | 重新连接 Composio 或修正端点设置。                                                                         |
| `voice/stt`                          | connect | 已配置语音转文字提供方是否就绪。                                                                                                              | 启用语音或配置受支持的语音转文字提供方。                                                                   |
| `voice/tts`                          | connect | 已配置文字转语音提供方是否就绪。                                                                                                              | 启用语音或配置受支持的文字转语音提供方。                                                                   |
| `search/<provider>`                  | connect | 每个已配置搜索提供方是否就绪。                                                                                                                | 配置发现项中指出的提供方密钥。                                                                             |
| `wallet/status`                      | connect | 已配置钱包组件是否就绪。                                                                                                                      | 安全配置钱包，或禁用不使用的钱包功能。                                                                     |
| `backend/api-url`                    | static  | 所选档案的托管后端 URL 是否有效。                                                                                                             | 为该档案设置有效的后端 API URL。                                                                           |
| `backend/reachable`                  | connect | 托管后端是否响应健康检查。                                                                                                                    | 检查配置的后端 URL 和网络访问。                                                                            |
| `backend/credential`                 | connect | 所选档案是否有可用的托管凭据。                                                                                                                | 通过受支持的认证流程重新登录。                                                                             |
| `backend/identity`                   | connect | 后端是否接受所选身份。                                                                                                                        | 检查租户身份和访问权限。                                                                                   |
| `backend/socket`                     | connect | 当前后端 socket 连接状态。                                                                                                                    | 检查 socket 服务和该档案的网络策略。                                                                       |
| `inference/managed-catalog`          | connect | 托管推理目录是否可访问。                                                                                                                      | 检查该档案的托管推理访问权限。                                                                             |
| `inference/model-in-catalog`         | connect | 配置的模型是否出现在其提供方目录中。不支持只读目录的提供方会跳过。                                                                            | 选择目录中的模型，或确认提供方支持目录查询。                                                               |
| `inference/auth-errors`              | connect | 记录的推理提供方认证错误。                                                                                                                    | 更新或重新连接提供方凭据。                                                                                 |
| `inference/live/<role>`              | live    | 每个配置推理角色是否能返回受上限约束的简短补全。无法在请求层强制输出上限的路由会跳过；相同路由共用一次请求。                                  | 跳过表示没有调用模型，也不表示提供方健康。对于已尝试的请求，按提示检查提供方、模型、凭据、额度和网络连接。 |
| `embeddings/live`                    | live    | 所选嵌入提供方返回的向量宽度是否符合配置。                                                                                                    | 检查提供方访问，并使模型与配置维度一致。                                                                   |

## 常见结果

- `pass` 表示检查成功。`warn` 表示需要留意的问题，但不会令报告的 `ok` 变为
  false。`error` 表示检查发现失败。`skipped` 会说明检查未运行的原因。
- 高于所选级别、受禁用组件影响，或使用无法强制输出上限的推理路由时，检查会被跳过。
  跳过不能说明服务健康。
- `doctor.models` 是单独的只读提供方目录报告，会区分可用目录、提供方不支持目录、
  访问失败和请求错误；它不会发送模型补全请求。
