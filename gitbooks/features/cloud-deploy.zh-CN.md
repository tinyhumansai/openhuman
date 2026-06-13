---
description: 在云端托管 headless openhuman-core——DigitalOcean App Platform、Fly.io 或任何 VPS 上的 Docker Compose。
icon: cloud
lang: zh-CN
---

# 云端部署

OpenHuman 是一个桌面应用，但它的 **Rust 核心**（`openhuman-core`）是一个可以托管在云端的 headless JSON-RPC 服务器。单独部署核心的用途包括：

- 多设备访问，让多个桌面客户端指向同一个托管核心
- 没有本地 Rust 工具链的内部测试人员
- 应该比笔记本 session 更长寿的长运行 cron job / webhook

本指南涵盖四条部署路径，由易到难：

1. [DigitalOcean App Platform：一键部署](#1-digitalocean-app-platform-一键部署)
2. [DigitalOcean App Platform：通过 doctl 手动部署](#2-digitalocean-app-platform-通过-doctl-手动部署)
3. [任何 VPS 通过 Docker Compose](#3-任何-vps-通过-docker-compose)
4. [Fly.io](#4-flyio)

每条路径部署的内容相同：一个运行 `openhuman-core serve` 在端口 `7788` 上的单一容器。公共主机应位于提供商的 TLS 之后，例如 `https://core.example.com/rpc`。仅限私有的主机——localhost、RFC1918 网络或 Tailscale 等 tailnet——可以使用纯 HTTP，例如 `http://100.x.x.x:7788/rpc`，当核心无法从公共互联网访问时。桌面应用已经知道如何与远程核心通信；在 `app/.env.local` 中设置 `OPENHUMAN_CORE_RPC_URL` 和 `OPENHUMAN_CORE_TOKEN=...`，然后启动即可。

---

## Bearer token 的单一事实来源

每次 `/rpc` 调用都携带 `Authorization: Bearer <token>`。核心在启动时通过两种方式加载该 token（[`src/core/auth.rs`](../../src/core/auth.rs)）：

1. **`OPENHUMAN_CORE_TOKEN` 环境变量**——由调用方预置（Tauri 壳层、Docker、App Platform、systemd unit 等）。核心原样使用此值，**绝不**写入文件。
2. **`{workspace}/core.token` 文件**——仅在 `OPENHUMAN_CORE_TOKEN` 未设置时由核心在首次启动时生成。独立运行的 `openhuman core run` 使用此方式，以便 CLI 客户端可以 `cat` 该文件。

**任何远程 / Docker 化部署的经验法则：始终设置 `OPENHUMAN_CORE_TOKEN`。** 不要在容器中依赖 `core.token`——临时文件系统会在重新部署时丢失它，任何试图从容器外部读取该文件的客户端都会得到过期或空值。这两条路径在启动时故意互斥；混合使用是"重新部署后 dashboard 报 401"的最常见原因。

要检查*运行中*的核心在使用什么，在主机上运行 [`scripts/print-core-token.sh`](../../scripts/print-core-token.sh)（或在容器内使用 `docker compose exec`）：

```bash
scripts/print-core-token.sh --where     # 打印 'env' 或 'file:/path'
scripts/print-core-token.sh --redact    # 前 8 个十六进制字符 + '…'（适合日志）
scripts/print-core-token.sh             # 完整值（直接管道到客户端）
```

桌面应用的首次运行选择器也在 Core RPC URL + token 字段旁暴露了一个**测试连接**按钮，它向该 URL 和输入的 token 触发 `core.ping`，并在持久化配置之前内联报告 `Connected ✓` / `Auth failed` / `Unreachable`。

---

## 开始前的准备工作

| 设置 | 必需 | 说明 |
| ---- | ---- | ---- |
| `OPENHUMAN_CORE_TOKEN` | 是 | 客户端发送给 `/rpc` 的 Bearer token。用 `openssl rand -hex 32` 生成。**任何持有此 token 的人都可以驱动核心。** |
| `BACKEND_URL` | 是 | 核心通信的 Tinyhumans 后端（生产环境为 `https://api.tinyhumans.ai`）。 |
| `OPENHUMAN_APP_ENV` | 否 | `production` 或 `staging`。默认 `production`。 |
| `OPENHUMAN_CORE_HOST` | 否 | 容器中默认 `0.0.0.0`。 |
| `OPENHUMAN_CORE_PORT` | 否 | 默认 `7788`。 |
| `RUST_LOG` | 否 | `info` 足够；`debug` 用于排查。 |

运行中的容器暴露的端点：

- `GET /health`，公共存活探针。每条部署路径的健康检查都使用它。
- `POST /rpc`，受 bearer 保护的 JSON-RPC 入口。
- `GET /events`、`GET /ws/dictation`，公共流式通道。

`OPENHUMAN_WORKSPACE` 目录（容器内为 `/home/openhuman/.openhuman`）保存核心的配置、sqlite 数据库和技能状态。**在每个生产部署中将其挂载到持久卷**，否则重启时会丢失数据。

---

## 1. DigitalOcean App Platform：一键部署

点击下面的按钮，从本仓库的 [`.do/app.yaml`](../../.do/app.yaml) 创建一个新的 App Platform 应用：

[![Deploy to DO](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/tinyhumansai/openhuman/tree/main)

然后，在 App Platform UI 中，**在首次部署完成之前**：

1. 打开 **Settings → App-Level Environment Variables** 标签页。
2. 将占位符 `OPENHUMAN_CORE_TOKEN` 值替换为强 secret（`openssl rand -hex 32`）。标记为 encrypted。
3. 如果部署的是 staging，将 `OPENHUMAN_APP_ENV` 改为 `staging`，`BACKEND_URL` 改为 `https://staging-api.tinyhumans.ai`。
4. 点击 **Save**。App Platform 用新 secret 重新部署。

App Platform 处理 TLS、崩溃重启、日志流式传输和 `git push` 时的滚动重新部署（在 `.do/app.yaml` 中设置 `deploy_on_push: true` 以选择加入）。

> **持久化说明：** App Platform Basic 不提供块存储。核心的工作区位于容器的临时文件系统中，重新部署时会丢失。如需持久存储，请附加托管数据库或升级到支持卷的套餐。参见 [Compose 路径](#3-任何-vps-通过-docker-compose)获取开箱即用持久卷的自助替代方案。

---

## 2. DigitalOcean App Platform：通过 doctl 手动部署

如果你不想点击 UI：

```bash
# 一次性：安装 doctl 并认证。
doctl auth init

# 编辑 .do/app.yaml - 将 OPENHUMAN_CORE_TOKEN 设置为真实值（或通过 --spec 配合 envsubst 在创建时传入）。然后：
doctl apps create --spec .do/app.yaml

# 观察构建：
doctl apps list
doctl apps logs <app-id> --type build --follow
```

编辑 spec 后更新现有应用：

```bash
doctl apps update <app-id> --spec .do/app.yaml
```

---

## 3. 任何 VPS 通过 Docker Compose

适用于任何安装了 Docker Engine ≥ 24 和 Compose 插件的主机。DigitalOcean Droplet、Hetzner、Linode、EC2、家用服务器。

每个生产版本都会向 GHCR 发布多标签镜像：

```bash
docker pull ghcr.io/tinyhumansai/openhuman-core:latest        # 追踪最新生产切版
docker pull ghcr.io/tinyhumansai/openhuman-core:v1.2.4        # 按 GitHub Release tag 固定
docker pull ghcr.io/tinyhumansai/openhuman-core:1.2.4         # 按 SemVer 固定
```

镜像是 `linux/amd64`。arm64 主机拉取独立 tarball，该 tarball 附在同一 GitHub Release 中（`openhuman-core-<version>-aarch64-unknown-linux-gnu.tar.gz`），或在 arm64 构建器上从源码构建镜像。

使用已发布镜像快速运行：

```bash
docker run -d --name openhuman-core -p 7788:7788 \
  -e OPENHUMAN_CORE_TOKEN="$(openssl rand -hex 32)" \
  -e BACKEND_URL=https://api.tinyhumans.ai \
  -e OPENHUMAN_APP_ENV=production \
  -v openhuman-workspace:/home/openhuman/.openhuman \
  ghcr.io/tinyhumansai/openhuman-core:latest
```

或使用仓库内的 Compose 文件（仍然从 `Dockerfile` 本地构建镜像；在 `docker-compose.yml` 中将 `image:` 字段切换为 `ghcr.io/tinyhumansai/openhuman-core:latest` 以改用已发布镜像）：

```bash
# 在服务器上：
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman

# 配置 secrets：
cp .env.example .env
# 编辑 .env - 至少填写：
#   BACKEND_URL=https://api.tinyhumans.ai
#   OPENHUMAN_CORE_TOKEN=<openssl rand -hex 32>
#   OPENHUMAN_APP_ENV=production

# 构建并启动：
docker compose up -d

# 验证：
docker compose ps
curl -fsS http://localhost:7788/health
```

### 无 Docker 的 Headless 安装

如果主机无法运行 Docker，抓取附在最新 [GitHub Release](https://github.com/tinyhumansai/openhuman/releases/latest) 中的独立 CLI tarball：

```bash
# 选择匹配你主机架构的 tarball。
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)  TARGET=x86_64-unknown-linux-gnu  ;;
  aarch64) TARGET=aarch64-unknown-linux-gnu ;;
  *) echo "Unsupported arch: $ARCH"; exit 1 ;;
esac
VERSION=1.2.4   # 设置为你想要的版本
curl -fsSL "https://github.com/tinyhumansai/openhuman/releases/download/v${VERSION}/openhuman-core-${VERSION}-${TARGET}.tar.gz" \
  | tar -xz -C /usr/local/bin
openhuman-core --version
```

然后在你选择的 service manager 下运行 `openhuman-core serve`（systemd、supervisord 等），使用上述相同的环境变量。

### Headless 自更新契约

Headless 部署应将 `openhuman.update_apply` 视为安全原语：它下载 release asset，将其原子地写入当前二进制文件旁边，然后返回。不会自动退出。

`openhuman.update_run` 遵循 `config.update.restart_strategy`：

- `self_replace`（默认）：stage 二进制文件，发布一个进程内重启请求，让运行中的核心自行 respawn。
- `supervisor`：stage 二进制文件并返回 `restart_requested=false`。你的外部 service manager 必须重启进程。

对于长运行的 Linux 服务，设置：

```toml
[update]
restart_strategy = "supervisor"
rpc_mutations_enabled = false
```

或等效的环境变量：

```bash
OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY=supervisor
OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED=false
```

推荐的 `systemd` 配置：

```ini
Restart=always
ExecReload=/bin/kill -HUP $MAINPID
```

运维流程：

1. 调用 `openhuman.update_check` 发现 release。
2. 在你的 `update.toml` 中配置 `restart_strategy = "supervisor"`（或设置 `OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY=supervisor`），以便核心 stage 新二进制文件而不尝试自行 re-exec，然后调用 `openhuman.update_apply` 或 `openhuman.update_run`。`restart_strategy` 是配置设置，不是 RPC 参数。
3. 显式重启 unit：`systemctl restart openhuman`。

如果下载或 staging 失败，运行中的二进制文件会保持原位，不会请求重启。如果 staged 二进制文件在重启后被证明有问题，通过你的包管理器、镜像标签或 release artifact 恢复之前的二进制文件，然后再次重启 supervisor。

Compose 文件（[`docker-compose.yml`](../../docker-compose.yml)）将核心映射到 `:7788`，挂载命名卷 `openhuman-workspace` 用于持久化，并设置 `restart: unless-stopped` 以便主机重启后核心自动恢复。

### 更新

```bash
git pull
docker compose build
docker compose up -d
```

对于暴露 RPC 的生产部署，建议禁用可变的 update RPC（`OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED=false`），并通过你现有的镜像标签或包管理流程执行 rollout。

### 日志

```bash
docker compose logs -f openhuman-core
```

### 轮换 bearer token

`OPENHUMAN_CORE_TOKEN` 是公共互联网与完整 RPC 访问之间的唯一屏障。按时间表轮换它，并在任何疑似泄露后轮换：

```bash
# 1. 生成新 token 并更新服务器端 .env。
openssl rand -hex 32 > /tmp/new-token
sed -i.bak "s|^OPENHUMAN_CORE_TOKEN=.*|OPENHUMAN_CORE_TOKEN=$(cat /tmp/new-token)|" .env
rm /tmp/new-token .env.bak

# 2. 重启容器，让新值到达核心进程。
docker compose up -d --force-recreate openhuman-core

# 3. 确认运行中的容器正在使用新 token（脱敏）。
docker compose exec openhuman-core /bin/sh -c \
  'echo -n "$OPENHUMAN_CORE_TOKEN" | head -c 8; echo "…"'

# 4. 更新每个桌面客户端（切换模式 → 在选择器中重新粘贴，或编辑 app/.env.local 中的 OPENHUMAN_CORE_TOKEN 然后重新启动）。仍然持有旧 token 的客户端会在下一次 /rpc 调用时收到 HTTP 401——这是预期行为，不是回归。
```

对于 App Platform，在 **Settings → App-Level Environment Variables** 中执行相同操作：编辑 `OPENHUMAN_CORE_TOKEN` secret，让 App Platform 重新部署。没有单独的 token 文件需要删除；环境变量是唯一的状态。

### 置于 TLS 之后

使用 Caddy、nginx 或 Traefik 作为 `:7788` 的反向代理。最小 `Caddyfile`：

```caddy
core.example.com {
  reverse_proxy localhost:7788
}
```

---

## 将桌面应用指向托管核心

在桌面应用的环境文件（`app/.env.local`）中：

```bash
# 使用托管核心而不是生成本地 sidecar。
OPENHUMAN_CORE_RUN_MODE=external
OPENHUMAN_CORE_RPC_URL=https://core.example.com/rpc
OPENHUMAN_CORE_TOKEN=<你在服务器上设置的相同 token>
```

对于没有公共 IP 的私有 tailnet-only VM，改用 tailnet URL：

```bash
OPENHUMAN_CORE_RUN_MODE=external
OPENHUMAN_CORE_RPC_URL=http://100.x.x.x:7788/rpc
OPENHUMAN_CORE_TOKEN=<你在服务器上设置的相同 token>
```

重启桌面应用。`App.tsx` 中的 provider 链会将所有 RPC 调用路由到远程核心；其他一切不变。公共 `http://` 主机被应用选择器拒绝；对任何可从公共网络访问的核心使用 HTTPS。

---

## 命名卷所有权与 Docker entrypoint

Docker 默认创建 `root:root` 拥有的命名卷。因为核心以非 root `openhuman` 用户（UID 10001）运行，banner 之后的首次写入——`init_rpc_token → write_token_file` 到 `$OPENHUMAN_WORKSPACE`——如果没有先修复所有权，会抛出 `Permission denied (os error 13)`。

镜像在 `/usr/local/bin/docker-entrypoint-core.sh` 附带一个专用 entrypoint，它：

1. 以 `root` 启动。
2. 对 `$OPENHUMAN_WORKSPACE` 和 `$HOME/.openhuman`（`OPENHUMAN_CORE_TOKEN` 未设置时 `core.token` 被写入的目录）运行 `mkdir -p` + `chown openhuman:openhuman`。
3. 调用 `exec gosu openhuman openhuman-core "$@"` 来降权并移交控制权给二进制文件。

这是**幂等的**：在新创建的卷上，chown 会修复 root 拥有的目录；在已经修复过的卷上，chown 是 no-op。从早于该修复的镜像升级时，不需要手动执行 `docker volume rm`。

该 entrypoint 名为 `docker-entrypoint-core.sh`，仅接入根 `Dockerfile`。E2E 镜像（`e2e/docker-entrypoint.sh`）不受影响。

---

## 4. Fly.io

[Fly.io](https://fly.io) 非常适合 `openhuman-core`：它自动处理 TLS，在所有套餐上支持持久卷，并且可以自动停止空闲机器以削减成本。

### 前置条件

- 安装并认证 [flyctl](https://fly.io/docs/flyctl/install/)（`fly auth login`）
- Fly.io 账户

### 步骤 1 —— 启动应用

```bash
fly launch --no-deploy --config .fly/fly.toml
```

Fly.io 自动检测 `Dockerfile`。选择靠近你用户的区域，并在提示时跳过首次部署。这会生成一个配置文件。

### 步骤 2 —— 配置 `.fly/fly.toml`

仓库在 [`.fly/fly.toml`](../../.fly/fly.toml) 附带一个模板。用 `fly launch` 期间选择的值填充 `<your-app-name>` 和 `<your-region>`：

```toml
app = '<your-app-name>'
primary_region = '<your-region>'

[build]
  dockerfile = "Dockerfile"

[env]
  OPENHUMAN_CORE_HOST = "0.0.0.0"
  OPENHUMAN_CORE_PORT = "7788"
  OPENHUMAN_WORKSPACE = "/home/openhuman/.openhuman"
  RUST_LOG = "info"

[[mounts]]
  source = "openhuman_workspace"
  destination = "/home/openhuman/.openhuman"

[http_service]
  internal_port = 7788
  force_https = true
  auto_stop_machines = 'stop'
  auto_start_machines = true
  # min_machines_running = 0 在空闲时完全停止机器（最便宜），但
  # 空闲后的第一个请求需要支付冷启动惩罚（容器启动 +
  # Rust 二进制文件初始化——数秒）。设为 1 可保持一台机器热备。
  min_machines_running = 0
  processes = ['app']

  [[http_service.checks]]
    interval = "30s"
    timeout = "5s"
    grace_period = "10s"
    method = "GET"
    path = "/health"

[[vm]]
  memory = '1gb'
  cpus = 1
```

### 步骤 3 —— 创建持久卷

```bash
fly volumes create openhuman_workspace --size 5 --region <your-region> --config .fly/fly.toml
```

**将工作区挂载到持久卷**，否则每次重新部署都会丢失数据。

### 步骤 4 —— 设置 secrets

```bash
# 必需
fly secrets set OPENHUMAN_CORE_TOKEN="$(openssl rand -hex 32)"
fly secrets set BACKEND_URL="https://api.tinyhumans.ai"
fly secrets set OPENHUMAN_APP_ENV="production"

# 建议——任何可公开访问的部署：
fly secrets set OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED="false"
fly secrets set OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY="supervisor"

# 可选——错误报告和 analytics：
fly secrets set OPENHUMAN_CORE_SENTRY_DSN="https://<key>@o<org>.ingest.sentry.io/<project>"
fly secrets set OPENHUMAN_ANALYTICS_ENABLED="true"
```

保存 `OPENHUMAN_CORE_TOKEN` 的值——你稍后需要它来连接桌面应用。**任何持有此 token 的人都可以驱动核心**；像密码一样对待它，并在任何疑似泄露后通过 `fly secrets set OPENHUMAN_CORE_TOKEN="$(openssl rand -hex 32)"` 轮换。

### 步骤 5 —— 部署

```bash
fly deploy --config .fly/fly.toml
```

验证核心健康：

```bash
curl -fsS https://<your-app-name>.fly.dev/health
```

### 步骤 6 —— 将桌面应用指向托管核心

在 `app/.env.local` 中：

```bash
OPENHUMAN_CORE_RUN_MODE=external
OPENHUMAN_CORE_RPC_URL=https://<your-app-name>.fly.dev/rpc
OPENHUMAN_CORE_TOKEN=<你在步骤 4 中设置的 token>
```

或使用桌面应用中的**首次运行选择器**（Core RPC URL + token 字段，带**测试连接**按钮）来配置，无需编辑文件。

### 持续部署

要在每次推送到 `main` 时自动重新部署，在 `.github/workflows/fly-deploy.yml` 添加 workflow 文件：

```yaml
name: Fly Deploy
on:
  push:
    branches:
      - main
    paths:
      - 'src/**'
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'Dockerfile'
      - '.fly/fly.toml'
      - 'scripts/docker-entrypoint-core.sh'
jobs:
  deploy:
    name: Deploy openhuman-core
    runs-on: ubuntu-latest
    concurrency: deploy-group
    steps:
      - uses: actions/checkout@v4
      # 将 Fly action 固定到带标签的 release（或完整 commit SHA），而不是 @master——
      # 追踪移动分支意味着信任未来推送到那里的每个 commit，包括被入侵的维护者账户所做的任何 commit。
      - uses: superfly/flyctl-actions/setup-flyctl@1.5
      - run: flyctl deploy --remote-only --config .fly/fly.toml
        env:
          FLY_API_TOKEN: ${{ secrets.FLY_API_TOKEN }}
```

用 `fly tokens create deploy` 生成部署 token，并将其作为名为 `FLY_API_TOKEN` 的仓库 secret 添加。

### 更新

```bash
fly deploy --config .fly/fly.toml
```

对于固定版本的部署，在 `.fly/fly.toml` 中更新镜像标签并重新部署：

```toml
[build]
  image = "ghcr.io/tinyhumansai/openhuman-core:v1.2.4"
```

### 日志

```bash
fly logs --config .fly/fly.toml
```

### 已知陷阱 —— 卷上的 UID 不匹配

如果你在从 `Dockerfile` 构建（创建 UID 10001 的 `openhuman` 用户）和拉取预构建的 GHCR 镜像（使用 UID 1000）之间切换，已写入持久卷的文件会被旧 UID 拥有，并在启动时产生 `Permission denied (os error 13)`。

通过 SSH 进入并重新拥有工作区来修复：

```bash
fly ssh console --config .fly/fly.toml
chown -R openhuman:openhuman /home/openhuman/.openhuman/
exit
fly machine restart --config .fly/fly.toml
```

---

## 冒烟测试

仓库附带 [`.github/workflows/deploy-smoke.yml`](../../.github/workflows/deploy-smoke.yml)，它在每次触及部署工件的 PR 上运行。它构建 Docker 镜像、启动它并轮询 `/health`，以便云部署路径中的回归在到达 `main` 之前就在 CI 中失败。

该 workflow 包含两个 job：

- **`docker-image`**——设置 `OPENHUMAN_CORE_TOKEN` 且不挂载卷。保护 DigitalOcean App Platform 路径（`.do/app.yaml`），其中 token 始终预置且不挂载持久卷。
- **`docker-volume-permissions`**——省略 `OPENHUMAN_CORE_TOKEN` 并在 `/home/openhuman/.openhuman` 挂载一个新的匿名卷。复现 issue #2065 的确切失败模式，并断言 `/health` 返回 200 且日志中不存在 `Permission denied (os error 13)`。

要在本地运行相同的检查：

```bash
docker build -t openhuman-core:smoke .

# Token-set 路径（App Platform）：
docker run -d --name oh-smoke -p 7788:7788 \
  -e OPENHUMAN_CORE_TOKEN=smoke-test-token \
  openhuman-core:smoke
curl -fsS http://localhost:7788/health
docker rm -f oh-smoke

# Fresh-volume / no-token 路径（Docker Compose、VPS）：
docker volume create oh-vol-test
docker run -d --name oh-vol-smoke -p 7789:7788 \
  -v oh-vol-test:/home/openhuman/.openhuman \
  openhuman-core:smoke
curl -fsS http://localhost:7789/health
docker rm -f oh-vol-smoke
docker volume rm oh-vol-test
```
