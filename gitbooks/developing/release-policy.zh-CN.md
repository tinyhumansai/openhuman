---
description: 发布节奏、版本策略、OAuth 与安装包规则。发布是如何运作的。
icon: ship
lang: zh-CN
---

# 发布策略：最新桌面构建与 OAuth

本 runbook 描述了我们如何避免用户在**过时的桌面安装包**上完成 **OAuth**（包括 **Gmail**），而规范流程始终要求**最新**发布版本。

## 分发

- [tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/releases) 的 **GitHub Releases** 是桌面构建的主要来源。
- **Tauri 更新器**端点（见 `scripts/prepareTauriConfig.js` 和发布工作流）应将用户指向当前发布产物。
- **淘汰旧稳定版产物：** 当弃用一条发布线时，在 **GitHub Releases** 上移除或隐藏过时的安装包资源，将 **网站 / CDN** 下载链接更新为 **releases/latest**（或当前版本），刷新**更新器 manifest**（例如 Gist / `latest.json`）使其不再指向已弃用的构建，并抽查旧直接 URL 在适当位置是否被**重定向、返回 404 或 410**。验证方式：尝试从文档或书签中已知的旧资源 URL，确认它们不再提供主要安装路径。

## OAuth 最低应用版本

生产 Web 构建在**构建时**嵌入一个**最低支持的应用 semver**，使 OAuth 深度链接无法在已弃用的二进制文件上完成。每个安装包携带构建时设定的 floor；对于从不升级的用户，提高 floor 需要他们安装一个**新**的发布版本（或通过应用内更新）。可选的未来工作：仅通过**运行时** API 强制执行移动的最低版本，捆绑值仅作为 fallback。

| 变量 | 用途 |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `VITE_MINIMUM_SUPPORTED_APP_VERSION` | 例如 `0.51.0` —— 桌面应用必须 **≥** 此版本才能完成 `openhuman://oauth/success`。 |
| `VITE_LATEST_APP_DOWNLOAD_URL` | 可选；默认为 `https://github.com/tinyhumansai/openhuman/releases/latest`。当门禁阻止 OAuth 时打开。 |

将这些配置为 **GitHub Actions 变量**。它们必须同时存在于独立的 **`pnpm build`** 步骤和 **`.github/workflows/build-desktop.yml`** 中的 **`tauri-apps/tauri-action`** 步骤环境变量中（由 `release-production.yml` / `release-staging.yml` 调用的可重用矩阵），以便嵌入已发布安装包的 Vite bundle 包含该门禁。本地开发时保持 `VITE_MINIMUM_SUPPORTED_APP_VERSION` **未设置**（门禁禁用）。

实现：`app/src/utils/oauthAppVersionGate.ts`、`app/src/utils/desktopDeepLinkListener.ts`。

## Gmail / Google Cloud OAuth

- Google Cloud Console 中的 **Redirect URIs** 必须匹配**当前**后端 + 隧道回调路径。
- 桌面 scheme（`openhuman://`）是稳定的；当 `VITE_MINIMUM_SUPPORTED_APP_VERSION` 设置时，**已安装的二进制文件**必须满足最低版本。

## 发布清单（避免回归）

1. 按照现有版本工作流提升 `app/package.json` 和 `app/src-tauri/tauri.conf.json`（以及根目录 `Cargo.toml` / core）的版本。
2. 当弃用对旧安装包的支持时，在该发布**之前**或**同时**将 **`VITE_MINIMUM_SUPPORTED_APP_VERSION`** 设置为新的 floor（仓库 Actions 变量 + 上述两个工作流步骤）。
3. 从用户可见表面（GitHub Release 资源、网站、CDN、更新器 feed）移除、重定向或淘汰旧稳定版安装包和陈旧**更新器**条目。确认已弃用的资源无法从默认安装/更新流程中访问。
4. 从 **releases/latest** 的全新安装上冒烟测试 **Gmail 连接**。
5. 完成[手动冒烟清单](../../docs/RELEASE-MANUAL-SMOKE.md)，然后将完成的签字块（逐字复制，每个已勾选项目保持勾选）粘贴到发布 PR 描述中，然后再打 tag。

## 工作流：staging vs. production

两个一等 GitHub Actions 工作流，每个环境一个。按意图选择，而非切换 flag。

| 工作流 | 分支 | 提升 | 推送的 Tags | 并发组 | 使用场景 |
| ------------------------------------------------------- | --------- | ------- | -------------------------- | ----------------------- | --------------------------------------------------------------------- |
| [`release-staging.yml`](../../.github/workflows/release-staging.yml) | `main` | 仅 `patch` | `v<version>-staging` | `release-staging` | 为 QA 切割 staging 构建。运行频繁；semver 移动范围窄。 |
| [`release-production.yml`](../../.github/workflows/release-production.yml) | `main` | `patch` / `minor` / `major`（仅在 `main_head` 上） | `v<version>` | `release-production` | 提升已验证的 staging tag，或从 `main` HEAD 热修。 |

两个流程使用的矩阵构建 / 签名 / Sentry-DIF / 产物上传流水线位于 [`.github/workflows/build-desktop.yml`](../../.github/workflows/build-desktop.yml) 中，作为 `workflow_call` 可重用工作流。上述两个顶层工作流拥有 ref 解析、版本提升、tagging 和发布/清理；构建本身是共享的。

### 切割 staging 构建

1. 通过 `workflow_dispatch` 从 `main` 运行 **Release (Staging)**。
2. 工作流在 `main` 上提升 `patch`，commit `chore(staging): vX.Y.Z`，推送分支，并在该 commit 上创建不可变的 `vX.Y.Z-staging` tag。
3. 构建矩阵从 **tag**（而非 main HEAD）运行，因此即使 `main` 已经前进，rerun 也会重建字节相同的内容。
4. 失败时 staging tag 会被自动删除；`main` 上的提升 commit 保留，因此下一次切割从 `vX.Y.(Z+1)` 继续。

没有单独的 `staging` 分支，staging 切割和 production 提升都存在于 `main` 上。两者仅通过 tag 后缀（`-staging` vs 无）和创建工作流来区分。

### 提升为 production（默认流程）

1. 通过 `workflow_dispatch` 以 `release_source = staging_tag`（默认）运行 **Release Production**。
2. 留空 `staging_tag` 以提升最新的 `v*-staging`，或传入显式 tag（例如 `v1.2.4-staging`）以固定版本。
3. 工作流去除 `-staging` 后缀，在同一 commit 上创建 `v<version>`，并从该 tag 运行 production 构建矩阵。**不再提升版本**，产物复用 staging 已验证的内容。

### 从 `main` HEAD 热修

1. 通过 `workflow_dispatch` 以 `release_source = main_head` 和所需的 `release_type`（`patch` / `minor` / `major`）运行 **Release Production**。
2. 工作流运行遗留的提升-and-tag 路径：在 `main` 上提升，commit `chore(release): vX.Y.Z`，推送，tag `vX.Y.Z`，构建。
3. 仅当需要不经过 staging 的 production-only 修复时才使用此路径。

### Tag 策略与回滚

- **命名。** Staging tag 使用 SemVer 预发布后缀 `-staging`（`v1.2.4-staging`），因此它们在排序上位于匹配的 production tag *之前*。提升到 production 时逐字去除后缀；两个 tag 之间捆绑安装包中嵌入的版本是相同的。
- **冲突。** 如果目标 tag 已存在于本地或 `origin` 上，两个工作流都会快速失败。通过删除陈旧 tag（仅限组织维护者）或跳过它来解决。
- **回滚（production）。** 失败的构建矩阵会触发 `cleanup-failed-release`，删除草稿 GitHub Release 和 `v<version>` tag。它从中提升的 staging tag 保持不变，修复后可以重新提升。
- **回滚（staging）。** 失败的 staging 构建会删除 `v<version>-staging` tag。`main` 上的提升 commit 保留；下一次 staging 切割从新的 patch 号继续，而不是重新使用它（我们接受 patch 号中的一个小"缺口"，而不是与并发合并竞争）。
- **谁可以删除 tag。** 与 `main` 相同的写入权限。工作流驱动的清理通过工作流的 token 使用 `actions/github-script` 运行删除（GitHub App token 仅由 `prepare-build` 用于提升 commit + tag 推送）；手动删除（`git push --delete origin <tag>`）需要同等的维护者权限。
