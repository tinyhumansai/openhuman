<h1 align="center">OpenHuman</h1>

<p align="center">
 <strong>最快、最省钱、最高效的开源智能体框架。在一台 10 美元的 VPS 上运行 500 多个智能体。</strong><br/>
 给普通人用的桌面应用，给开发者用的 Rust 库。
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Latest Release" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="License" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="Public benchmarks" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">文档</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust 快速入门</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">基准测试</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">讨论区</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel（创建者）</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="OpenHuman 桌面应用演示" />
</p>

<p align="center">
	<a href="https://trendshift.io/repositories/23680" target="_blank">
		<img src="https://trendshift.io/api/badge/repositories/23680" alt="tinyhumansai%2Fopenhuman | Trendshift" width="250" height="55"/>
	</a>
	<a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
		<img alt="OpenHuman on Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=daily&amp;t=1778916022823">
	</a>
	<a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
		<img alt="OpenHuman on Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=weekly&amp;t=1779351403565">
	</a>
</p>

<p align="center">
  🇺🇸 <a href="../README.md">English</a> | 🇸🇦 <a href="./README.ar.md">العربية</a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md">한국어</a> | 🇩🇪 <a href="./README.de.md">Deutsch</a> | 🇹🇷 <a href="./README.tr.md">Türkçe</a> | 🇵🇰 <a href="./README.ur-pk.md">اردو</a>
</p>

> [!NOTE]
> 🎉 上线仅一周，OpenHuman 就连续九天成为 **GitHub 趋势榜第一的仓库**。

> **早期测试版。** OpenHuman 仍在积极开发中，难免有不完善的地方。

---

## 安装

最简单的方式是从 [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) 或 [最新发布页](https://github.com/tinyhumansai/openhuman/releases/latest) 下载桌面应用。macOS 提供 `.dmg`，Windows 提供 `.msi` 或 `.exe`，Linux 提供 `.deb` 或 `.AppImage`。

更喜欢用终端？安装脚本会为你的系统选择合适的安装包，校验其校验和并完成安装：

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

想先看看 macOS 和 Linux 脚本会做什么，可以在命令末尾加上 `bash -s -- --dry-run`。其他选项和故障排查见 [INSTALL.md](../INSTALL.md)。

---

## 为什么选择 OpenHuman？

大多数智能体框架给每个智能体开一个沉重的进程，并且每次调用都重新发送一大段提示词。OpenHuman 用少得多的资源完成同样的工作。它是唯一功能丰富、专为大规模智能体集群设计的开源框架：一台 10 美元的服务器就能装下 500 个智能体。

<table>

<tr>

<td width="50%" valign="top">

<h3>快</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">基准测试结果</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">冷启动数据</a></p>

<p>完成编码任务只需约 20 秒，是我们测试的七款 AI 智能体工具中最快的。启动只需十分之一秒。</p>

</td>

<td width="50%" valign="top">

<h3>省钱</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">成本与 token 数据</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">压缩原理</a></p>

<p>token 用量比一般的智能体工具少 2.6 倍，在我们的测试中总花费最低。</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>大规模下依然高效</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">集群测量数据</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">测试方法</a></p>

<p>内存和 CPU 用量约为一般智能体工具的八分之一。在一台 10 美元的服务器上运行 500 多个智能体。</p>

</td>

<td width="50%" valign="top">

<h3>为开发者而建</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust 快速入门</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/embedding">嵌入指南</a> · <a href="../crates/openhuman-embed/examples">示例</a></p>

<p>把它当作 Rust 库来用：像调用普通函数一样调用智能体，或者在一台小服务器上运行整个集群。</p>

</td>

</tr>

</table>


<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes vs OpenHuman" />
</p>

<p align="center"><em>相同的提示词，并排对比：“用动漫故事的形式给我写一个关于智能体框架的故事，写进一个 HTML 页面并帮我打开。”<br/>OpenHuman 用时 20 秒，使用 15k token，花费 $0.0054。Hermes 用时 9 分 40 秒，使用 37k token，花费 $0.0082。</em></p>

---

## 主要创新

大多数智能体框架只是一个简单的循环：把所有内容发给模型，等待，再重复。这对单个智能体可行，但很快就会变得又慢又贵，一旦同时运行几百个就会崩溃。

OpenHuman 重新设计了开销最大的几个环节：AI 要读多少文字、如何找到合适的工具、功能如何加载，以及整个系统需要多少机器资源。下面六个想法就是上文速度和节省的来源。每张卡片都链接到文档，方便你了解细节。

<table>

<tr>

<td width="50%" valign="top">

<h3>RLM token 压缩</h3>

<p><a href="https://arxiv.org/abs/2512.24601">RLM 论文</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">工作原理</a></p>

<p>基于 <a href="https://arxiv.org/abs/2512.24601">递归语言模型（Recursive Language Models）</a>。体积大的工具结果会在 AI 读取之前先被压缩。对于特别大的结果，AI 拿到的是一个可以搜索的句柄，而不是把全部内容读一遍。不会丢弃任何内容。</p>

</td>

<td width="50%" valign="top">

<h3>Jev：即时、准确的工具搜索</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">测量数据</a></p>

<p>Jev 是一个小模型，能从 1,215 个工具中找出合适的那一个。正确的工具出现在它推荐前几名的概率是 86.8%，而关键词搜索只有 70.5%。</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>统一的 Rust 总线</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">插件如何工作</a></p>

<p>每个功能，比如搜索、文档或语音，都接入同一条 Rust 总线，这个想法借鉴自 <a href="https://www.freedesktop.org/wiki/Software/dbus/">Linux 系统总线</a>。功能只在需要时才加载，其中一个卡住了，其余的仍能正常工作。</p>

</td>

<td width="50%" valign="top">

<h3>深度集成的记忆</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">记忆如何工作</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">记忆引擎</a></p>

<p>记忆是内置的。每一轮对话之前，OpenHuman 只挑出重要的内容，控制在 token 预算之内，并附上引用交给 AI。开箱即用，你也可以通过一个设置换成别的记忆引擎。</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>即时的浏览器与桌面控制</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">浏览器与电脑控制</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>智能体会使用真实的浏览器和你的桌面应用。Jev 根据屏幕上的按钮来决定每一次点击，不需要截图。遇到任何付款操作它都会先停下。</p>

</td>

<td width="50%" valign="top">

<h3>可编程的 Rust 核心</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust 快速入门</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">性能</a></p>

<p>整个框架是在单个进程中运行的编译型 Rust，所以启动只需十分之一秒，而且一直很轻量：在我们的编码任务中峰值仅 68 MB。同一个核心也是一个库。你可以在自己的 Rust 代码里调用智能体，也可以在一台小服务器上并排运行几百个。</p>

</td>

</tr>

</table>

---

## 基准测试

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench Verified 运行 swe-x86-1：OpenHuman 与其他六个框架在十个面板上的对比" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

我们让七个框架在相同的 SWE-bench 任务上运行，使用相同的模型、API 密钥和容器，并记录了每一次调用。测试环境和结果都公开在 [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks)，任何人都可以重新运行。最新一次运行是 [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md)，共十个任务。

OpenHuman 完成任务用时不到中位数的一半，token 少用 2.6 倍，内存和 CPU 只用八分之一。它发起了 114 次模型调用，其他框架是 134 到 212 次；整次运行花费 $0.05，其他框架花费 $0.08 到 $0.35。

它的优势来自每一步做得更少。核心是单进程的编译型 Rust，而不是 Node 或 Python，所以占用内存很少，在两次模型调用之间几乎不占 CPU。它发送的提示词是七个框架里最小的，包含 20 个工具也只有 4.6k token，因此每次调用更便宜、返回更快（中位数 1.65 秒，同样最快）。它得出答案所需的调用次数也更少，大部分节省正是来自这里。

| 每个已解决任务 | OpenHuman（对比中位数） | 其他六个的中位数 | 其他六个中最好的 |
| --- | --- | --- | --- |
| 耗时 (p50) | **19.8 s**（快 2.3 倍） | 46.5 s | 28.4 s (OpenCode) |
| Token | **167k**（少 2.6 倍） | 435k | 370k (Codex) |
| 成本 | **$0.0077**（-36%） | $0.012 | $0.0077（OpenCode，持平） |
| 峰值内存 | **68 MB**（少 7.7 倍） | 523 MB | 123 MB (Codex) |
| CPU 时间 | **1.3 s**（少 8 倍） | 10.4 s | 2.9 s (DeepSeek Harness) |
| 系统提示词 | **4.6k token**（-40%） | 7.7k | 6.2k (DeepSeek Harness) |

---

## 面向用户

如果你用过 Claude Code、Codex、OpenClaw 或 Hermes，这些概念你一定不陌生：带工具的智能体循环、MCP、技能、BYOK 模型和记忆。OpenHuman 全都有，可以在桌面应用、终端应用或无界面服务器中使用。

| 能力 | OpenHuman 提供什么 |
| --- | --- |
| 可配置的记忆 | [覆盖你的文件、代码仓库、信息流和应用的内置记忆](https://tinyhumans.gitbook.io/openhuman/features/memory)，每一轮之前带引用地调出，使用你选择的记忆引擎 |
| 语音智能体 | 可以随时打断的[实时语音智能体](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice)，另有语音输入和语音回复 |
| 电脑与浏览器控制 | [操作真实的 Chrome 浏览器和你的桌面应用](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer)，做任何无法撤销的操作前都会先询问 |
| 搜索 | [搜索引擎和搜索智能体](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search)：内置 Exa 和 Gemini，使用你自己的密钥还可用 Brave、Tavily、Parallel 等，提供带引用的有依据回答和 Deep Research |
| 工具 | [原生工具](https://tinyhumans.gitbook.io/openhuman/features/native-tools)：Shell 和编码、网页抓取、文档、图像和视频生成、定时任务 |
| MCP 与技能 | [MCP 服务器和技能包](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| OAuth 集成 | [通过 Composio 接入 119 个应用](https://tinyhumans.gitbook.io/openhuman/features/integrations)，并有[触发器](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers)，在事件发生时启动智能体 |
| 模型 | [本地模型（Ollama、LM Studio、MLX）和 27 个提供商的 BYOK](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models)，并支持[自动模型路由](https://tinyhumans.gitbook.io/openhuman/features/model-routing) |
| 渠道 | [14 个消息渠道](https://tinyhumans.gitbook.io/openhuman/features/channels)可作为智能体的前端：Telegram、Discord、iMessage、电子邮件等 |
| 工作流 | [持久化的工作流图](https://tinyhumans.gitbook.io/openhuman/features/workflows)：定时、事件或手动触发，审批步骤，暂停后可继续 |
| 安全 | [审批关卡](https://tinyhumans.gitbook.io/openhuman/features/approval-gate)、[沙箱执行](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security)（系统隔离或 Docker）、仅本地运行的[隐私模式](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode)、存放在[系统密钥环](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage)中的密钥 |
| 用量追踪 | [每次调用的成本和 token 用量](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage)，以及可回放的运行日志 |

从[快速上手](https://tinyhumans.gitbook.io/openhuman/overview/getting-started)或[指南](https://tinyhumans.gitbook.io/openhuman/guides)开始吧。

---

## 面向开发者

OpenHuman 是一个以库为先的框架。把它加入你的 Rust 项目，就能像调用普通函数一样调用智能体，不需要运行任何边车进程或守护进程。一个运行时可以容纳几百个智能体，每个智能体都有自己的模型、工具、记忆和沙箱。500 个智能体就能装进一台 10 美元的 VPS，所以每位客户一个智能体的产品不需要集群也能起步。

```rust
use openhuman_embed::{Access, Harness, Provider, Workspace};

let agent = Harness::builder()
    .provider(Provider::openai_compatible("https://api.openai.com/v1", "sk-...").model("gpt-5"))
    .workspace(Workspace::Ephemeral)
    .access(Access::readonly())
    .build()
    .await?;

let reply = agent.run("Summarize what you can see in this directory.").await?;
println!("{}", reply.reply);
```

接下来看：[Rust 快速入门](https://tinyhumans.gitbook.io/openhuman/developing/quickstart)、[嵌入指南](https://tinyhumans.gitbook.io/openhuman/developing/embedding)和[开发者文档](https://tinyhumans.gitbook.io/openhuman/developing)。

---

## 与其他框架相比如何？

下面把 OpenHuman 和大多数人已经在用的框架放在一起比较。前四行来自我们的[公开基准测试](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md)：十个 SWE-bench 编码任务，每个框架使用相同的模型和容器。每个数字都是每个已解决任务的数据。其余各行比较功能。

简单来说，其他框架很适合一个人搭配一个智能体工作。OpenHuman 同样可以做到，并且它还是可以作为库嵌入、并能扩展到整个集群的开源选择。产品变化很快，做决定前请先查看各个项目。

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **任务用时中位数**      | 37.8 s               | 36.1 s               | 62 s                 | 58.5 s               | 🚀 19.8 s                                 |
| **每个任务平均 token**  | 428k                 | 370k                 | 442k                 | 482k                 | 🚀 167k                                   |
| **每个任务峰值内存**    | 231 MB               | 123 MB               | 1.57 GB              | 816 MB               | 🚀 68 MB                                  |
| **每个任务平均成本**    | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **开源**                | 🚫 专有              | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **智能体集群**          | ⚠️ 每个智能体一个进程 | ⚠️ 每个智能体一个进程 | ⚠️ 每个智能体一个进程 | ⚠️ 每个智能体一个进程 | 🚀 10 美元 VPS 上 500 个智能体            |
| **可嵌入的库**          | ⚠️ CLI 之上的 SDK    | ⚠️ CLI 之上的 SDK    | 🚫 无                | ⚠️ Python 包         | 🚀 有类型的 Rust API                     |
| **记忆**                | ⚠️ 记忆文件          | ⚠️ 记忆文件          | ⚠️ 依赖插件          | ✅ 自我学习          | 🚀 每一轮都调用，并附引用                |
| **集成**                | ✅ MCP               | ✅ MCP               | ⚠️ 自行接入          | ⚠️ 自行接入          | 🚀 119 个 OAuth 应用、MCP、技能          |
| **消息渠道**            | 🚫 无                | 🚫 无                | ✅ 很多              | ✅ 若干              | ✅ 14 个，包括电子邮件                   |
| **浏览器与桌面**        | ⚠️ 通过 MCP 使用浏览器 | ⚠️ 通过 MCP 使用浏览器 | ✅ 浏览器            | ✅ 浏览器            | ✅ 浏览器和桌面应用                      |
| **工作流**              | 🚫 无                | 🚫 无                | ⚠️ 脚本              | ⚠️ 脚本              | 🚀 可视化，由智能体起草                  |
| **模型选择**            | ⚠️ Anthropic 模型    | ⚠️ 优先 OpenAI       | ✅ 任意              | ✅ 任意              | ✅ 任意，内置路由                        |

---

## 参与贡献

请阅读 [`CONTRIBUTING.md`](../CONTRIBUTING.md)，或者用[这段提示词](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you)让 AI 编码智能体带你上手。

1. 安装 Git、Node.js 24+、pnpm 10.10.0、Rust 1.96.1（含 `rustfmt` 和 `clippy`）、CMake、Ninja、ripgrep，以及你所用平台的桌面构建依赖。
2. Fork 并克隆仓库。运行 `git submodule update --init --recursive`，然后运行 `pnpm install`。
3. 做界面开发运行 `pnpm dev`，做桌面应用运行 `pnpm dev:app`。提交 PR 之前，请运行 `pnpm typecheck`、`pnpm format:check` 和 `cargo check --manifest-path Cargo.toml`。

更多内容见[环境搭建](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up)、[`AGENTS.md`](../AGENTS.md)和 [crates 概览](../crates/README.md)。OpenHuman 的许多部分位于 [`vendor/`](../vendor) 下各自独立的仓库中，同样欢迎贡献。

贡献者可获得免费周边，并在 [Discord](https://guild.tinyhumans.ai/) 上获得特别权限。

## Star 历史

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

## 贡献者

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
