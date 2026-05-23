---
lang: zh-CN
---

# Polymarket 集成（读取 + 交易）

本文档描述 issue #1398 的 Polymarket 集成。

## 范围

`polymarket` 工具现在支持以下 API 上的市场浏览和交易工作流：

- Gamma API (`https://gamma-api.polymarket.com`)
- CLOB API (`https://clob.polymarket.com`)

支持的读取操作：

- `list_markets`
- `get_market`
- `list_events`
- `get_orderbook`
- `get_price`
- `get_positions`
- `get_balance`
- `get_open_orders`
- `get_usdc_allowance`

支持的写入操作：

- `place_order`
- `cancel_order`

## 架构

实现位于 `src/openhuman/tools/impl/network/polymarket.rs`，辅助模块包括：

- `clob_auth.rs`：L1 凭据派生 + L2 HMAC 头
- `polymarket_orders.rs`：EIP-712 订单类型数据签名

关键运行时行为：

- Layer-2 API 凭据在首次认证调用时派生并缓存。
- 派生凭据持久化到 `integrations.polymarket.derived_clob_credentials`（在 secret-store 迁移落地前使用明文配置 fallback）。
- 下单前获取 `GET /nonce?user=<eoa>` 以避免重放/nonce 不匹配。
- USDC.e 授权通过 Polygon `eth_call` 对 ERC-20 `allowance(owner, spender)` 进行读取。

## 认证与签名流程

### L1 握手（一次性引导）

- 使用 Polygon chain id `137` 签署 CLOB `ClobAuth` EIP-712 payload。
- 调用 `POST /auth/api-key`；如需，fallback 到 `GET /auth/derive-api-key`。
- 持久化返回的 `{ apiKey, secret, passphrase }` 以供 L2 使用。

### L2 认证请求

每个认证的 CLOB 请求签署：

- `timestamp + method + request_path (+ POST 的 body)`

Headers：

- `POLY_ADDRESS`
- `POLY_SIGNATURE`
- `POLY_TIMESTAMP`
- `POLY_NONCE: 0`
- `POLY_API_KEY`
- `POLY_PASSPHRASE`

### 订单签名

`place_order` 使用以下 domain 签署 EIP-712 订单：

- name: `Polymarket CTF Exchange`
- version: `1`
- chain id: `137`
- verifying contract: `integrations.polymarket.clob_exchange_contract`

## 权限

写入操作目前由显式的临时审批 flag 保护。

- `place_order` 和 `cancel_order` 需要 `approved=true`。
- 如果省略或 `false`，工具返回：
  - `Polymarket write requires explicit user approval. Re-invoke with arguments.approved = true after confirming with the user.`

这是临时的，直到 #1339 的共享审批门禁集成进来。

## 配置

配置路径：`integrations.polymarket`。

字段：

- `enabled`（默认 `false`）
- `gamma_base_url`（默认 `https://gamma-api.polymarket.com`）
- `clob_base_url`（默认 `https://clob.polymarket.com`）
- `timeout_secs`（默认 `15`）
- `eoa_address`（可选默认用户地址）
- `polygon_rpc_url`（默认 `https://polygon-rpc.com`）
- `usdc_contract`（默认 `0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174`）
- `clob_exchange_contract`（默认 `0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E`）
- `derived_clob_credentials`（可选缓存的 L2 凭据）

## USDC Allowance 合约

`get_usdc_allowance` 仅报告授权状态；不改变链上状态。

- Token：Polygon 上的 USDC.e (`0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174`)
- Spender：Polymarket exchange (`0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E`)

如果授权不足，必须单独执行审批（wallet 工具 / 显式用户审批流程）。

## 错误与重试行为

- 4xx 错误视为客户端错误，不重试。
- 429 和 5xx 错误视为瞬态错误，最多重试 3 次。
- 退避固定为每次重试间隔 500ms。
- 超时表现为显式的 deadline 错误。

## 测试策略

单元测试位于 `src/openhuman/tools/impl/network/polymarket_tests.rs` 及辅助模块测试中。

- 现有读取路径和重试行为测试保持覆盖。
- 新增认证读取操作、写入审批门禁和 Polygon 授权读取的覆盖。
- `clob_auth.rs` 测试覆盖 HMAC/头 fixture 行为。
- `polymarket_orders.rs` 测试覆盖 domain 和确定性签名 fixture 行为。
