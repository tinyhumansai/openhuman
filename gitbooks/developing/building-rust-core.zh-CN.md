---
description: 在全新机器上从头构建 Rust 核心。
icon: terminal
lang: zh-CN
---

# 构建 Rust 核心

本页面向贡献者，是在全新机器上编译 Rust 核心的参考文档。

它仅涵盖**仓库根目录的 crate**：

- Cargo 包：`openhuman`
- 二进制文件：`openhuman-core`
- 库：`openhuman_core`

如果你需要完整的桌面应用（`pnpm dev`、Tauri、CEF、前端工具链），请使用[环境搭建](getting-set-up.zh-CN.md)。该路径有额外的 JavaScript、子模块和桌面运行时依赖，**不**需要用于纯核心的 `cargo` 工作流。

## 1. 安装指定版本的 Rust 工具链

仓库在 [`rust-toolchain.toml`](../../rust-toolchain.toml) 中固定了 Rust 版本：

- Channel：`1.93.0`
- Components：`rustfmt`、`clippy`

推荐安装方式：

```bash
rustup toolchain install 1.93.0 --component rustfmt --component clippy
rustup default 1.93.0
```

你也可以在安装 `rustup` 后，让 `cargo` 从 `rust-toolchain.toml` 自动安装。

## 2. 克隆仓库

仅核心开发：

```bash
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman
```

这对根目录 crate 来说已足够。

桌面/Tauri 开发则不同：

- 只有在构建桌面壳层或 CEF 感知的 Tauri 工具链时，才需要 `app/src-tauri/vendor/` 子模块。
- 该流程请遵循[环境搭建](getting-set-up.zh-CN.md)并运行 `git submodule update --init --recursive`。

## 3. 构建命令

从仓库根目录运行：

```bash
# 快速依赖 + 类型检查
cargo check --manifest-path Cargo.toml

# 实际 CLI / RPC 二进制文件的 Debug 构建
cargo build --manifest-path Cargo.toml --bin openhuman-core

# Release 构建
cargo build --manifest-path Cargo.toml --release --bin openhuman-core

# Rust 测试
cargo test --manifest-path Cargo.toml
```

注意：

- **包**名是 `openhuman`，但可运行的二进制文件是 **`openhuman-core`**。
- 如果你更喜欢面向包的 cargo 命令用于打包脚本，请使用 `-p openhuman`。
- 构建好的二进制文件位于 `target/debug/openhuman-core` 或 `target/release/openhuman-core`。

## 4. macOS 前置条件

安装：

- Xcode Command Line Tools：`xcode-select --install`

原因：

- `whisper-rs` 在构建期间编译原生代码。
- 在 macOS 上，该 crate 在 [`Cargo.toml`](../../Cargo.toml) 中以 `metal` 特性启用构建，因此需要 Apple 工具链和 SDK 头文件。

安装 Xcode CLT 后，核心应该能用上述 cargo 命令构建。

## 5. Linux 前置条件

### 仅核心包集合

在全新 Linux 机器上运行 `cargo` 前，先安装这些包。

**Ubuntu / Debian：**

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential cmake pkg-config clang libssl-dev libclang-dev \
  libasound2-dev libxi-dev libxtst-dev libxdo-dev libudev-dev \
  libstdc++-14-dev
```

**Arch Linux：**

```bash
sudo pacman -S --needed base-devel cmake pkgconf clang openssl \
  alsa-lib libxi libxtst xdotool libevdev
```

> 在 Arch 上，`clang` 包含 `libclang`，`base-devel` 包含 `gcc`（提供 `libstdc++`），因此不需要单独的 `-dev` 包。

这些包的重要性：

- `build-essential` / `base-devel`、`cmake`、`pkg-config` / `pkgconf`：传递性 Rust 依赖使用的原生构建。
- `clang`、`libclang-dev`：bindgen / C 和 C++ 编译路径，被原生 crate 使用。
- `libssl-dev` / `openssl`：某些网络依赖需要的 OpenSSL 头文件。
- `libasound2-dev` / `alsa-lib`、`libxi-dev` / `libxi`、`libxtst-dev` / `libxtst`、`libxdo-dev` / `xdotool`、`libudev-dev`（Arch 中已包含在 `systemd-libs` 内）、`libevdev`：被核心构建引入的音频/输入/设备 crate 所需。

### `whisper-rs` + `clang` 注意事项

`whisper-rs-sys` 在 `clang` 下可能会失败并提示：

```text
fatal error: 'array' file not found
```

这就是为什么文档特别指出 `libstdc++-14-dev`：`clang` 在 Ubuntu runner 上可能会选择 GCC 14 的 C++ 头文件。

如果你的发行版布局仍然导致构建无法解析 `libstdc++.so`，请使用 [`AGENTS.md`](../../AGENTS.md) 中记录的相同变通方案：

```bash
# Ubuntu/Debian —— 按需调整 GCC 版本
sudo ln -sf /usr/lib/gcc/x86_64-linux-gnu/13/libstdc++.so /usr/lib/x86_64-linux-gnu/libstdc++.so
```

Arch Linux 通常不需要此变通方案，因为 `gcc-libs` 将 `libstdc++.so` 放在了默认库搜索路径上。

### Linux 桌面/Tauri 包集合

如果你构建的是桌面壳层而非仅核心 crate，请安装更广泛的依赖集合。

**Ubuntu / Debian**（镜像自 [`.github/workflows/build-desktop.yml`](../../.github/workflows/build-desktop.yml)）：

```bash
sudo apt-get update
sudo apt-get install -y \
  libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  patchelf cmake libasound2-dev libxdo-dev libxtst-dev libx11-dev libxi-dev \
  libevdev-dev libssl-dev libclang-dev \
  libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 \
  libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 \
  libgbm1 libpango-1.0-0 libcairo2 libatspi2.0-0 libxshmfence1 libu2f-udev
```

**Arch Linux：**

```bash
sudo pacman -S --needed gtk3 webkit2gtk-4.1 libayatana-appindicator \
  librsvg patchelf nss nspr at-spi2-core libcups libdrm \
  libxkbcommon libxcomposite libxdamage libxfixes libxrandr \
  mesa pango cairo libxshmfence
```

仅在需要 `app/src-tauri/` 时使用桌面列表；对于根 crate 工作，上面较小的仅核心列表是相关的基线。

## 6. Windows 前置条件

安装：

- 通过 `rustup` 安装 Rust
- Visual Studio Build Tools 2022 或带 **使用 C++ 的桌面开发** 工作负载的 Visual Studio
- CI 和发布构建使用的 MSVC 目标：`x86_64-pc-windows-msvc`

安装 Microsoft 工具链后推荐的命令：

```powershell
rustup toolchain install 1.93.0 --component rustfmt --component clippy
rustup target add x86_64-pc-windows-msvc
cargo build --manifest-path Cargo.toml --bin openhuman-core
```

Windows 注意事项：

- 仓库对 `whisper-rs-sys` 打补丁以强制使用静态 MSVC CRT，并避免 [`Cargo.toml`](../../Cargo.toml) 中提到的 `LNK2038` / `LNK1169` 不匹配。请使用 MSVC 工具链，而非 MinGW。

## 7. 相关路径

- [环境搭建](getting-set-up.zh-CN.md)：完整的桌面贡献者设置，含 `pnpm`、Tauri、子模块和 sidecar staging。
- [OpenHuman 架构](architecture/README.zh-CN.md)：核心在桌面应用和 RPC 流程中的位置。
