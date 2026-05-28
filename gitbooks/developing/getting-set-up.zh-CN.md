---
description: 如何从源码构建 OpenHuman —— 工具链、vendored Tauri CLI 和本地桌面构建。
icon: wrench
lang: zh-CN
---

# 构建与安装 OpenHuman

本指南涵盖完整的桌面/源码安装路径和发布安装包。

如果你只需要在新机器上运行仓库根目录的 Rust crate，请使用[构建 Rust 核心](building-rust-core.zh-CN.md)。该页面记录了固定的 Rust 工具链、OS 包前置条件以及 `openhuman-core` 的精确 `cargo` 命令。

本指南涵盖两条路径：

1. 从源码构建并编译 OpenHuman
2. 安装最新的稳定发布二进制文件

## 前置条件

- `git`
- Node.js 24 或更高版本（见 `app/package.json`）
- `pnpm@10.10.0`（见根目录 `package.json` 的 `packageManager` 字段）
- 通过 `rustup` 安装的 Rust 1.93.0，含 `rustfmt` 和 `clippy`（见 `rust-toolchain.toml`）
- CMake，原生 Rust 依赖所需
- `app/src-tauri/vendor/` 下的 Git 子模块，vendored CEF-aware Tauri CLI 所需
- 平台桌面构建工具：macOS 上的 Xcode Command Line Tools，或 Linux 上的 Tauri GTK/WebKit/AppIndicator 包集合

macOS Homebrew 快速开始：

```bash
brew install node@24 pnpm rustup-init cmake
rustup toolchain install 1.93.0 --profile minimal
rustup component add rustfmt clippy --toolchain 1.93.0
```

Arch Linux 快速开始：

```bash
sudo pacman -S --needed nodejs npm rustup cmake base-devel clang openssl \
  alsa-lib xdotool libxtst libxi libevdev gtk3 webkit2gtk-4.1 \
  libayatana-appindicator librsvg patchelf nss nspr at-spi2-core \
  libcups libdrm libxkbcommon libxcomposite libxdamage libxfixes \
  libxrandr mesa pango cairo libxshmfence
npm install -g pnpm@10.10.0
rustup toolchain install 1.93.0 --profile minimal
rustup component add rustfmt clippy --toolchain 1.93.0
```

## 从源码构建（本地编译）

从仓库根目录运行：

```bash
# 1) 克隆并进入仓库
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman

# 2) 获取 vendored Tauri/CEF 源码
git submodule update --init --recursive

# 3) 安装 JS 依赖（workspace）
pnpm install

# 4) 构建桌面应用产物
pnpm build
```

本地开发（而非生产构建）：

```bash
# 仅 Web UI 开发
pnpm dev

# 使用 vendored Tauri/CEF CLI 的桌面应用开发：从 workspace 根目录运行
pnpm --filter openhuman-app dev:app
```

## 安装最新稳定版（macOS/Linux x64）

主要安装命令：

```bash
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

安装器行为：

- 解析你平台的最新稳定 OpenHuman 发布版本
- 可用时验证产物摘要
- 本地安装（默认不需要 sudo）
- macOS：将 `OpenHuman.app` 安装到 `~/Applications`
- Linux x64：将 AppImage 安装为 `~/.local/bin/openhuman` 并写入桌面入口

实用 flag：

```bash
# 预览操作而不写入文件
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash -s -- --dry-run
```

## Windows（最新稳定版）

使用 PowerShell：

```powershell
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

Windows 安装器行为：

- 解析最新稳定版
- 下载 x64 的 MSI/EXE
- 可用时验证摘要
- 在安装包支持的情况下执行按用户安装

## ARM Linux 构建（aarch64）

ARM Linux 构建由于 CEF 和 GTK 依赖需要特殊处理。

### 前置条件

```bash
# 安装 xvfb 用于 headless 构建/测试
sudo apt install xvfb
```

### 构建

```bash
cd app
pnpm tauri build --target aarch64-unknown-linux-gnu
```

### 运行 ARM 二进制文件

该二进制文件需要设置 CEF 库路径：

### 选项 1 —— 直接调用

```bash
REL_DIR=app/src-tauri/target/aarch64-unknown-linux-gnu/release
CEF_DIR=$(ls -d "$REL_DIR"/build/cef-dll-sys-*/out/cef_linux_aarch64 2>/dev/null | head -n1)
export LD_LIBRARY_PATH="$CEF_DIR:$REL_DIR/deps:$REL_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
"$REL_DIR/OpenHuman" --no-sandbox
```

### 选项 2 —— Wrapper 脚本（推荐）

保存到 `~/bin/openhuman` 并赋予可执行权限（`chmod +x ~/bin/openhuman`）：

```bash
#!/bin/bash
REL_DIR=/path/to/app/src-tauri/target/aarch64-unknown-linux-gnu/release
CEF_DIR=$(ls -d "$REL_DIR"/build/cef-dll-sys-*/out/cef_linux_aarch64 2>/dev/null | head -n1)
export LD_LIBRARY_PATH="$CEF_DIR:$REL_DIR/deps:$REL_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$REL_DIR/OpenHuman" --no-sandbox "$@"
```

### DEB 包安装

```bash
DEB_FILE=$(ls app/src-tauri/target/aarch64-unknown-linux-gnu/release/bundle/deb/OpenHuman_*_arm64.deb | head -n1)
sudo dpkg -i "$DEB_FILE"
```

### GTK 初始化修复

ARM 构建需要 GTK 在 Tauri 创建系统托盘之前初始化。这在 `vendor/tauri-cef/crates/tauri-runtime-cef/src/lib.rs` 中处理：

```rust
// CEF 初始化后，添加：
#[cfg(target_os = "linux")]
{
    gtk::init().ok();
}
```

如果托盘初始化失败并提示 "GTK has not been initialized"，请确保此修复已到位后重新构建。

全平台手动下载链接：

- 网站：https://tinyhuman.ai/openhuman
- 最新发布：https://github.com/tinyhumansai/openhuman/releases/latest

## 故障排除

### macOS：`pnpm dev:app` 退出并提示 "CEF cache is held by another OpenHuman instance"

**症状**

`pnpm dev:app`（或 Tauri 壳层的任何 debug 构建）在窗口出现前退出，提示类似：

```text
[openhuman] CEF cache at /Users/<you>/Library/Caches/com.openhuman.app/cef is held by another OpenHuman instance (host <hostname>, pid 12345).
Quit the running instance and try again.
Workaround:
  pkill -f "OpenHuman.app/Contents"
  pkill -f "openhuman-core"
```

**原因**

CEF（Chromium Embedded Framework）通过 `~/Library/Caches/com.openhuman.app/cef` 下的 `SingletonLock` 符号链接对其用户数据目录持有独占锁。已安装的 `.app` 包和开发二进制文件使用相同的标识符（`com.openhuman.app`），因此它们无法并排运行。如果没有 preflight，`cef::initialize` 会返回失败，而 vendored `tauri-runtime-cef` 会以 Rust 回溯和无可操作消息的方式 panic（这是 preflight 落地前的 issue #864）。

**修复**

退出另一个 OpenHuman 实例并重新运行。最快路径：

```bash
pkill -f "OpenHuman.app/Contents"
pkill -f "openhuman-core"
pnpm dev:app
```

如果锁是由崩溃进程留下的（PID 已不存在），preflight 会自动移除陈旧的 `SingletonLock`，开发启动将继续，无需手动清理。

**已知限制**

开发和发布构建仍然共享 `com.openhuman.app` 作为缓存标识符。将开发隔离到单独的 `com.openhuman.app.dev` 缓存需要修改 vendored `tauri-runtime-cef`（缓存路径在运行时内部从 bundle 标识符构建，未暴露给 openhuman 壳层）。作为 #864 的后续跟踪。

### 核心端口上的陈旧 `openhuman` RPC 进程

**症状**

之前的 Tauri 构建或 `openhuman-core run` harness 在 `OPENHUMAN_CORE_PORT`（默认 `7788`）上留下了一个监听进程。在 issue #1130 之前，新的 Tauri 构建会静默附加到该监听器，导致版本漂移，以及新构建的 `OPENHUMAN_CORE_TOKEN` 不匹配时出现 401。

**当前行为（issue #1130）**

`core_process::ensure_running` 现在在启动时探测端口：

- 如果 `GET /` 将监听器识别为 OpenHuman 核心（JSON body 含 `"name": "openhuman"`），则将其视为之前运行的陈旧进程并主动终止（Unix 上 `SIGTERM`，750ms 后 `SIGKILL`；Windows 上 `taskkill /F /T /PID`）。Tauri 主机随后会生成自己的全新嵌入式核心。
- 如果监听器是其他东西（或不讲 HTTP），启动会大声失败，并在日志中显示冲突，而非静默附加。
- 设置 `OPENHUMAN_CORE_REUSE_EXISTING=1` 以选择回到遗留的 attach-to-anything 行为，在将 `openhuman-core run` 作为手动调试 harness 运行时很有用。

**手动清理（仍然有效）**

```bash
pkill -f "OpenHuman.app/Contents"
pkill -f "openhuman-core"
```
