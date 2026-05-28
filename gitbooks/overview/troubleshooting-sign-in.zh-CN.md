---
description: >-
  诊断登录失败、OAuth 回调无法完成以及远程核心 RPC 认证问题。
icon: key
lang: zh-CN
---

# 登录故障排查

当社交登录卡住、返回欢迎界面，或核心日志中出现未授权的 `/auth` 请求时，使用此 checklist。

## 检查后端可达性

从桌面应用所在的同一网络，验证公共 OpenHuman 端点：

```bash
curl -I https://tinyhumans.ai/
curl -I https://api.tinyhumans.ai/health
```

如果网站能加载但 API 端点失败，桌面应用可能无法将 OAuth 回调兑换为 session。在 issue 报告中记录 HTTP 状态码、区域和 DNS 结果。

## 检查所选核心

如果你使用**高级**远程核心模式，在开始 OAuth 之前确认 RPC URL 和 bearer token：

```bash
curl -sS https://your-core.example/rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer CORE_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}'
```

`401` 响应表示桌面 token 与远程核心 token 不匹配。在重试 Google 或 GitHub 登录之前先修复这个问题。

## 检查深度链接回调

成功的桌面 OAuth 以 `openhuman://auth?...` 回调结束。如果浏览器显示了该 URL 但应用仍停留在欢迎界面：

1. 确保只运行了一个 OpenHuman 桌面实例。
2. 重启应用，保持相同的远程核心设置，并重试登录。
3. 如果使用远程核心，检查核心是否收到 `openhuman.auth_store_session`。

对于远程核心，临时手动注入可以确认核心本身是正常的：

```bash
curl -sS https://your-core.example/rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer CORE_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.auth_store_session","params":{"token":"JWT_FROM_CALLBACK"} }'
```

不要把真实 JWT 粘贴到公共 GitHub issue 中。对 token 进行脱敏处理，只附加状态码、主机名、应用版本、操作系统和相关日志行。

## Bug 报告中应包含的内容

* 应用版本和操作系统。
* 核心模式是本地还是远程。
* RPC URL 主机、脱敏后的 token 状态和 `core.ping` 结果。
* 使用的 OAuth 提供商。
* 浏览器中是否出现了 `openhuman://auth` URL。
* 如果存在，第一条未授权日志行。
