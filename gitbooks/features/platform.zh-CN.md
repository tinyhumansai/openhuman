---
description: >-
  OpenHuman 以什么形式交付（原生 React + Tauri v2 桌面应用，Rust core）、
  支持的平台，以及当前范围内的功能。
icon: layer-plus
---

# 平台与可用性

OpenHuman 是一个原生桌面应用，不是浏览器扩展，也不是 Electron 包装器。基于 **React + Tauri v2** 构建，搭载 **Rust core**，它体积小、启动快、不干扰你的工作流。

***

## 支持的平台

| 平台       | 架构                    | 分发方式                   |
| ---------- | ---------------------- | -------------------------- |
| **macOS**  | Intel、Apple Silicon   | `.dmg` 安装包、Homebrew     |
| **Windows**| x64、ARM64             | `.msi` 安装包              |
| **Linux**  | x64                    | AppImage、`.deb`           |

***

## 为什么是原生应用

OpenHuman 作为原生应用构建而非 Web 包装器，有三个原因：

**体积小。** 只有典型通信工具的几分之一。不到一秒启动，内存占用极少。

**启动快。** 无需初始化浏览器引擎。立即就绪接受请求。

**操作系统级安全。** 凭据保存在你平台的安全密钥链中：macOS Keychain、Windows Credential Manager、Linux Secret Service。敏感数据永不放在浏览器存储或明文文件中。本地记忆树的 SQLite 数据库位于你的工作区文件夹中，由你拥有。

***

## 架构概览

```text
┌──────────────────────────────────────────────────┐
│ Tauri shell - windowing, OS integration │
└──────────────────────────────────────────────────┘
 │ JSON-RPC ↕
┌──────────────────────────────────────────────────┐
│ Rust core（进程内 `openhuman` core）│
│ • Memory Tree, integrations, auto-fetch │
│ • Model router, TokenJuice, native tools │
│ • Voice (STT in, TTS out, Meet agent) │
└──────────────────────────────────────────────────┘
 │
┌──────────────────────────────────────────────────┐
│ React frontend - screens, navigation │
└──────────────────────────────────────────────────┘
```

Shell 是载体（负责窗口化、进程生命周期、IPC）。所有产品逻辑都在 Rust core 中。React 前端通过 JSON-RPC 与 core 通信。参见[架构](../developing/architecture/)获取完整图景。

***

## 实时通信

桌面应用与 OpenHuman 后端保持持久连接。响应在生成时流式输出；输出渐进出现，而非等待后的最终结果。如果网络断开，应用会自动重连，使用渐进退避。

***

## 离线行为

你的本地状态保存在设备上。偏好设置、设置和连接的源配置在离线时仍然可用。本地记忆树完全可访问，你可以浏览 [Obsidian 存储库](obsidian-wiki/)，在无网络连接的情况下阅读你现有的笔记。

自动拉取和实时 LLM 调用需要网络连接。网络恢复时，下一个 20 分钟触发周期会从上次停止的地方继续。

***

## 自动更新

桌面 shell 通过 Tauri 的更新插件自动更新，针对 GitHub Releases 上发布的一份清单。进程内 OpenHuman core 打包在同一 bundle 中，所以 shell 更新会同时升级两者。
