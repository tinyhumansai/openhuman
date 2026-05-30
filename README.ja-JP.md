<h1 align="center">OpenHuman</h1>

<p align="center">
 <img src="./gitbooks/.gitbook/assets/demo.png" alt="The Tet" />
</p>

<p align="center" style="display: inline-block">
 <a href="https://trendshift.io/repositories/23680" target="_blank" style="display: inline-block">
  <img src="https://trendshift.io/api/badge/repositories/23680" alt="tinyhumansai%2Fopenhuman | Trendshift" style="width: 250px; height: 55px;" width="250" height="55"/>
 </a> 
 &nbsp;
 <a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
  <img alt="OpenHuman - An open source AI harness built with the human in mind | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=daily&amp;t=1778916022823">
 </a>
 
</p>
 
<p align="center">
 <strong>OpenHuman はあなたのパーソナル AI スーパーインテリジェンスです：ローカルメモリ、必要に応じてマネージドサービス、シンプルで強力。</strong>
</p>


<p align="center">
 <a href="https://discord.tinyhumans.ai/">Discord</a> •
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> •
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X/Twitter</a> •
 <a href="https://tinyhumans.gitbook.io/openhuman/">ドキュメント</a> •
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel（作者）をフォロー</a>
</p>

<p align="center">
  🇺🇸 <a href="./README.md">English</a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md">한국어</a> | 🇩🇪 <a href="./README.de.md">Deutsch</a>
</p>


<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="最新リリース" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="./LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="ライセンス" /></a>
 <a href="./README.md"><img src="https://img.shields.io/badge/lang-English-blue" alt="English" /></a>
 <a href="./README.zh-CN.md"><img src="https://img.shields.io/badge/lang-简体中文-blue" alt="简体中文" /></a>
 <a href="./README.ko.md"><img src="https://img.shields.io/badge/lang-한국어-blue" alt="한국어" /></a>
 <a href="./README.de.md"><img src="https://img.shields.io/badge/lang-Deutsch-blue" alt="Deutsch" /></a>
</p>

> **早期ベータ版**: 現在も活発に開発中です。荒削りな部分があることをご了承ください。

> **ローカル + マネージドサービス、upfront:** OpenHuman は Memory Tree、Obsidian スタイルの Markdown ヴォルト、ワークスペース設定、およびローカルランタイム状態をあなたのマシン上に保存します。デフォルトのマネージド体験では、アカウントサインイン、モデルルーティング、Web 検索プロキシ、および Composio コネクタレイヤーを介したマネージド統合/OAuth フローに、OpenHuman ホスト型サービスが引き続き使用されます。独自のモデル、検索、または Composio 認証情報を持ち込みたい場合は、カスタム/ローカル設定を選択してください。一部のリアルタイムトリガーおよびホスト型機能には、マネージドバックエンドが引き続き必要です。

インストールや利用開始は、ウェブサイト [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) からダウンロードするか、以下のコマンドを実行してください。

```bash
# DMG や EXE は https://tinyhumans.ai/openhuman からダウンロードするか、ターミナルから実行してください

# macOS または Linux x64 の場合
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash

# Windows の場合
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

<!-- TODO: translate (ja-JP) — English source mirrored from README.md so non-EN readers get the same install caveats. Please translate. -->
> **Linux:** the AppImage can crash on launch under Wayland (and on Arch-based distros with `sharun: Interpreter not found!`) — see [#2463](https://github.com/tinyhumansai/openhuman/issues/2463) for the cause and env-var workarounds.
Arch Linux package maintainers can use the [`openhuman-bin` AUR recipe](./packages/arch/openhuman-bin/);
once published, Arch users can install it with `yay -S openhuman-bin`.
<!-- /TODO -->

# OpenHuman とは?

OpenHuman は、あなたの日常生活に統合されるよう設計されたオープンソースのエージェント型アシスタントです。各項目は[ドキュメント](https://tinyhumans.gitbook.io/openhuman/)内の詳細な解説にリンクしています。

- **シンプル、UI ファースト、そしてヒューマン** クリーンなデスクトップ体験と短いオンボーディングパスで、インストールから動作するエージェントまで数クリックで到達できます。設定優先のセットアップも、ターミナルも不要です。エージェントには[顔があります](https://tinyhumans.gitbook.io/openhuman/features/mascot): 喋り、周囲に反応し、実際の参加者として[あなたの Google Meet に参加](https://tinyhumans.gitbook.io/openhuman/features/mascot/meeting-agents)し、数週間にわたってあなたのことを覚えており、あなたが入力をやめてもバックグラウンドで考え続けるデスクトップマスコットです。

- **[118+ のサードパーティ統合](https://tinyhumans.gitbook.io/openhuman/features/integrations) と [自動取得](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki/auto-fetch)**: Gmail、Notion、GitHub、Slack、Stripe、Calendar、Drive、Linear、Jira などのスタックに **ワンクリック OAuth** で接続できます。すべての接続は型付きツールとしてエージェントに公開され、20 分ごとにコアがアクティブな各接続を巡回し、最新データを[メモリーツリー](https://tinyhumans.gitbook.io/openhuman/features/integrations/auto-fetch)に取り込みます。プロンプトも、自分で書くポーリングループも不要なので、エージェントは今朝の時点で明日のコンテキストを既に持っています。

  マネージド統合は OpenHuman の Composio コネクタレイヤーを使用します。OAuth ハンドシェイクおよび統合ツール呼び出しは、デフォルトでマネージドバックエンドを介してプロキシされます。代わりに Composio を直接実行したい場合は、独自の Composio API キーでダイレクトモードを構成してください。リアルタイムトリガーの Webhook は、その後あなたがホストして配線する必要があります。

- **[Memory Tree](https://tinyhumans.gitbook.io/openhuman/features/memory-tree) + [Obsidian Wiki](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki)**: あなたのデータとアクティビティから構築されるローカルファーストのナレッジベースです。接続したすべての情報は ≤3k トークンの Markdown チャンクへ正規化され、スコアリングされ、階層的なサマリーツリーに畳み込まれて **あなたのマシン上の SQLite** に保存されます。同じチャンクは Obsidian 互換のボルトに `.md` ファイルとして配置され、開いて閲覧・編集できます。Karpathy 氏の [obsidian-wiki ワークフロー](https://x.com/karpathy/status/2039805659525644595)にインスパイアされています。

- **電池同梱(Batteries included)**: ウェブ検索、ウェブフェッチ用[スクレイパー](https://tinyhumans.gitbook.io/openhuman/features/native-tools)、フルコーダーツールセット(ファイルシステム、git、lint、test、grep)、そして[ネイティブ音声](https://tinyhumans.gitbook.io/openhuman/features/voice)(STT 入力、ElevenLabs TTS 出力、マスコットのリップシンク、ライブ Google Meet エージェント)がデフォルトで組み込まれています。デフォルトで、[モデルルーティング](https://tinyhumans.gitbook.io/openhuman/features/model-routing)は OpenHuman バックエンドを使用して各ワークロードに適切な LLM(reasoning、fast、または vision)を選択およびプロキシします。一つのサブスクリプションですべてのモデルが含まれます。「ファイル読み込みのためにプラグインをインストール」という煩わしさはありません。デバイス上のワークロード向けに [Ollama によるオプショナルなローカル AI](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-ai) も利用できます。

- **[スマートトークン圧縮 (TokenJuice)](https://tinyhumans.gitbook.io/openhuman/features/token-compression)**: すべてのツール呼び出し、スクレイプ結果、メール本文、検索ペイロードは、LLM モデルに渡される前にトークン圧縮レイヤーを通過します。HTML は Markdown に変換され、長い URL は短縮され、冗長なツール出力は設定可能なルールレイヤーで重複排除と要約が行われるなど…。CJK、絵文字などのマルチバイト文字は書記素(grapheme)単位で完全に保持され、除去されることはありません。同じ情報をわずかなトークン数で得られます。コストとレイテンシを最大 80% 削減します。

- **[メッセージングチャネル](https://tinyhumans.gitbook.io/openhuman/features/integrations#messaging-channels)** と **[プライバシー & セキュリティ](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security)**: あなたが既に使っているチャネル全体での送受信が可能で、ワークフローデータはデバイス上に留まり、ローカルで暗号化され、あなた自身のものとして扱われます。

## ソースからのコントリビュート

新しいコントリビューターの方は、まず [`CONTRIBUTING.md`](./CONTRIBUTING.md) で fork/PR ワークフローとローカル検証コマンドを確認してください。最短経路は以下のとおりです:

1. Git、Node.js 24+、pnpm 10.10.0、Rust 1.93.0(`rustfmt` + `clippy`)、CMake、Ninja、ripgrep、プラットフォーム向けデスクトップビルドの前提条件をインストールします。
2. リポジトリを fork してクローンし、`pnpm install` の前に `git submodule update --init --recursive` を実行して、ベンダー化された Tauri/CEF のソースを取得します。
3. ウェブのみの UI 作業には `pnpm dev` を、デスクトップシェルには `pnpm --filter openhuman-app dev:app` を使用し、PR を出す前に `pnpm typecheck`、`pnpm format:check`、`cargo check -p openhuman --lib` などの集中チェックを実行してください。

詳細なドキュメント: [アーキテクチャ](https://tinyhumans.gitbook.io/openhuman/developing/architecture) · [セットアップガイド](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up) · [クラウドデプロイ](./gitbooks/features/cloud-deploy.md)。

## コンテキストを数週間ではなく数分で

OpenHuman は、数分であなたのことを理解する初めてのエージェントハーネスです。[Karpathy 氏の LLM ナレッジベース](https://x.com/karpathy/status/2039805659525644595)にインスパイアされました。ほとんどのエージェントは冷えた状態から始まります。Hermes はあなたの作業を見て学習し、OpenClaw はプラグインがコンテキストを運び込むのを待ちます。いずれにせよ、エージェントがあなたのスタックを十分理解して本当に役立つようになるまで、数日から数週間を費やすことになります。

<p align="center">
 <img src="./gitbooks/.gitbook/assets/image (1).png" alt="OpenHuman のコンテキスト構築図">
</p>

> OpenHuman はあなたのすべてのドキュメント、メール、チャットを要約・圧縮し、エージェントがあなたについてすべてを覚えていられるメモリーグラフを作成します。

OpenHuman はその待ち時間をスキップします。アカウントを接続し、[自動取得](https://tinyhumans.gitbook.io/openhuman/features/integrations/auto-fetch)に 20 分ループでローカルにデータを取得させ、その後 [Memory Trees](https://tinyhumans.gitbook.io/openhuman/features/memory-tree) ですべてを Markdown ファイルに圧縮し、[Karpathy 流の Obsidian wiki](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki) にインテリジェントに保存します。

たった 1 回の同期パスで、エージェントはあなたの受信箱、カレンダー、リポジトリ、ドキュメント、メッセージの完全な(圧縮された)コンテキストを得ます。トレーニング期間も「数週間お待ちください」もありません。エージェントはあなたになり、あなたによって制御されます。

既に他のコーディングエージェント間で [agentmemory](https://github.com/rohitg00/agentmemory) をセルフホストしていますか? OpenHuman にはそれにプロキシするオプションの `Memory` バックエンドが同梱されています。`config.toml` で `memory.backend = "agentmemory"` を設定すれば、同じ永続ストアが Claude Code、Cursor、Codex、OpenCode と並んで OpenHuman を駆動します。セットアップ方法は [agentmemory バックエンド](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki/agentmemory-backend)のページを参照してください。

## OpenHuman と他のエージェントハーネスの比較

ハイレベルな比較です(製品は進化するため、各ベンダーで確認してください)。OpenHuman は **ベンダーの乱立を最小限に抑え**、**ワークフロー知識をデバイス上に保ち**、チャットだけでなくあなたのデータに対する **永続的なメモリ** をエージェントに与えるよう構築されています。

|                      | Claude Cowork       | OpenClaw            | Hermes Agent        | OpenHuman                                          |
| -------------------- | ------------------- | ------------------- | ------------------- | -------------------------------------------------- |
| **オープンソース**   | 🚫 プロプライエタリ | ✅ MIT              | ✅ MIT              | ✅ GNU                                             |
| **開始が簡単**       | ✅ デスクトップ + CLI | ⚠️ ターミナル中心   | ⚠️ ターミナル中心   | ✅ クリーンな UI、数分                             |
| **コスト**           | ⚠️ サブスク + アドオン | ⚠️ モデル持ち込み   | ⚠️ モデル持ち込み   | ✅ 1 つのサブスク + TokenJuice                     |
| **メモリ**           | ✅ チャット範囲のみ | ⚠️ プラグイン依存   | ✅ 自己学習         | 🚀 Memory Tree + Obsidian ボルト、オプションの [agentmemory](https://github.com/rohitg00/agentmemory) バックエンド |
| **統合**             | ⚠️ 少数のコネクター | ⚠️ 持ち込み         | ⚠️ 持ち込み         | 🚀 OAuth 経由で 118+                               |
| **自動取得**         | 🚫 なし             | 🚫 なし             | 🚫 なし             | ✅ 20 分同期でメモリに取り込み                     |
| **API の乱立**       | 🚫 追加キー         | 🚫 BYOK             | 🚫 マルチベンダー   | ✅ 1 アカウント                                    |
| **モデルルーティング** | 🚫 単一モデル       | ⚠️ 手動             | ⚠️ 手動             | ✅ ビルトイン                                      |
| **ネイティブツール** | ✅ コードのみ       | ✅ コードのみ       | ✅ コードのみ       | ✅ コード + 検索 + スクレイパー + 音声             |

# GitHub でスターをお願いします

_AGI と人工意識への道を進んでいますか? リポジトリにスターをつけて、他の人にも道筋を見つけてもらいましょう。_

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

# コントリビューター・ホール・オブ・フェイム

愛を示して、殿堂入りしましょう。コントリビューターには無料グッズと [Discord](https://discord.tinyhumans.ai/) への特別アクセスが提供されます。

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
