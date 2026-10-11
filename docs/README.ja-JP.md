<h1 align="center">OpenHuman</h1>

<p align="center">
 <strong>最速、最安、最も効率的なオープンソースのエージェントハーネス。月10ドルのVPS1台で500以上のエージェントを動かせます。</strong><br/>
 一般の方にはデスクトップアプリ。開発者にはRustライブラリ。
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="最新リリース" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="ライセンス" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="公開ベンチマーク" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">ドキュメント</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust クイックスタート</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">ベンチマーク</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">ディスカッション</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel（作者）</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="OpenHuman デスクトップアプリの紹介" />
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
> 🎉 公開から1週間以内に、OpenHuman は9日連続で **GitHub のトレンド1位のリポジトリ** になりました。

> **早期ベータ版。** OpenHuman は活発に開発中のため、荒削りな部分があります。

---

## インストール

一番簡単なのは、[tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) または[最新リリース](https://github.com/tinyhumansai/openhuman/releases/latest)からデスクトップアプリをダウンロードする方法です。macOS 用は `.dmg`、Windows 用は `.msi` または `.exe`、Linux 用は `.deb` または `.AppImage` です。

ターミナルを使いたい方へ。インストールスクリプトが環境に合ったパッケージを選び、チェックサムを確認してからインストールします。

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

macOS と Linux のスクリプトが何を行うかを事前に確認するには、コマンドの末尾に `bash -s -- --dry-run` を付けます。その他のオプションとトラブルシューティングは [INSTALL.md](../INSTALL.md) にあります。

---

## なぜ OpenHuman なのか

多くのエージェントハーネスは、エージェントごとに重いプロセスを1つ動かし、呼び出しのたびに大きなプロンプトを送り直します。OpenHuman は同じ仕事をはるかに少ない資源でこなします。機能が豊富で、大規模なエージェント群のために作られた唯一のオープンソースハーネスです。月10ドルのサーバー1台に500のエージェントが収まります。

<table>

<tr>

<td width="50%" valign="top">

<h3>速い</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">ベンチマーク結果</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">コールドスタートの数値</a></p>

<p>コーディングタスクを約20秒で終えます。テストした7つの AI エージェントツールの中で最速です。起動は0.1秒です。</p>

</td>

<td width="50%" valign="top">

<h3>安い</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">コストとトークンのデータ</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">圧縮のしくみ</a></p>

<p>一般的なエージェントツールよりトークン使用量が2.6分の1で、テストでの合計費用も最も低くなりました。</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>大規模でも効率的</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">エージェント群の計測</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">測定方法</a></p>

<p>一般的なエージェントツールに比べ、メモリと CPU の使用量は約8分の1です。月10ドルのサーバーで500以上のエージェントを動かせます。</p>

</td>

<td width="50%" valign="top">

<h3>開発者のために</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust クイックスタート</a> · <a href="../gitbooks/developing/embed/README.md">組み込みガイド</a> · <a href="../crates/openhuman-embed/examples">サンプル</a></p>

<p>Rust ライブラリとして使えます。エージェントを普通の関数のように呼び出すことも、小さなサーバー1台でエージェント群をまるごと動かすこともできます。</p>

</td>

</tr>

</table>


<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes と OpenHuman の比較" />
</p>

<p align="center"><em>同じプロンプトを並べて比較: 「エージェントハーネスについての物語をアニメ風に書いて、HTML ページにして開いてください。」<br/>OpenHuman は20秒で完了し、15kトークン、$0.0054でした。Hermes は9分40秒かかり、37kトークン、$0.0082でした。</em></p>

---

## 主なイノベーション

多くのエージェントハーネスは単純なループです。すべてをモデルに送り、待ち、これを繰り返します。エージェントが1つならこれで動きますが、すぐに遅く高くなり、数百を動かすと破綻します。

OpenHuman は、最もコストのかかる部分を見直しました。AI が読むテキストの量、適切なツールの見つけ方、機能の読み込み方、全体に必要なマシンの大きさです。以下の6つの工夫が、上で紹介した速さと節約の源です。詳しく知りたい場合は、各カードからドキュメントに進めます。

<table>

<tr>

<td width="50%" valign="top">

<h3>RLM によるトークン圧縮</h3>

<p><a href="https://arxiv.org/abs/2512.24601">RLM の論文</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">しくみ</a></p>

<p><a href="https://arxiv.org/abs/2512.24601">Recursive Language Models</a> に基づいています。大きなツールの結果は、AI が読む前に圧縮されます。非常に大きいものは、AI が全部を読む代わりに検索できるハンドルを受け取ります。捨てられる情報はありません。</p>

</td>

<td width="50%" valign="top">

<h3>Jev: 即座で正確なツール検索</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">計測結果</a></p>

<p>Jev は、1,215個の中から適切なツールを見つける小さなモデルです。正解が上位候補に入る割合は86.8%で、キーワード検索の70.5%を上回ります。</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>統一された Rust バス</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">プラグインのしくみ</a></p>

<p>検索、ドキュメント、音声などの機能はすべて、1本の Rust バスにつながります。これは <a href="https://packages.debian.org/stable/dbus">Linux のシステムバス</a>から借りた考え方です。機能は必要なときだけ読み込まれ、1つが止まっても他は動き続けます。</p>

</td>

<td width="50%" valign="top">

<h3>深く統合されたメモリ</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">メモリのしくみ</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">メモリエンジン</a></p>

<p>メモリは最初から組み込まれています。毎ターンの前に、OpenHuman はトークンの予算内で重要なものだけを選び、出典付きで AI に渡します。そのまま使え、設定を変えるだけで別のメモリエンジンに切り替えられます。</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>ブラウザとデスクトップの即時操作</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">ブラウザとコンピューターの操作</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>エージェントは実際のブラウザとデスクトップアプリを使います。Jev は画面上のボタンから次のクリックを選ぶので、スクリーンショットは不要です。支払いの前には必ず止まります。</p>

</td>

<td width="50%" valign="top">

<h3>プログラムできる Rust コア</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust クイックスタート</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">パフォーマンス</a></p>

<p>ハーネス全体が1つのプロセスで動くコンパイル済みの Rust なので、0.1秒で起動し、軽いままです。コーディングタスクでのピークは68 MBでした。同じコアがライブラリにもなります。自分の Rust コードからエージェントを呼び出すことも、小さなサーバー1台で数百のエージェントを並べて動かすこともできます。</p>

</td>

</tr>

</table>

---

## ベンチマーク

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench Verified の実行 swe-x86-1: OpenHuman と他の6つのハーネスを10のパネルで比較" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

7つのハーネスを、同じモデル、同じ API キー、同じコンテナで同じ SWE-bench のタスクに対して実行し、すべての呼び出しを計測しました。セットアップと結果は [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) で公開しているので、誰でも再実行できます。最新の実行は [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md) で、タスク数は10です。

OpenHuman は、中央値の半分未満の時間でタスクを終え、トークンは2.6分の1、メモリと CPU は8分の1でした。モデル呼び出しは114回で、他は134回から212回でした。実行全体の費用は$0.05で、他は$0.08から$0.35でした。

勝因は、1ステップあたりの仕事が少ないことです。コアは Node や Python ではなく、1つのプロセスで動くコンパイル済みの Rust なので、メモリ使用量が小さく、モデル呼び出しの合間の CPU 使用もほぼありません。20個のツールを含めても4.6kトークンと、7つの中で最小のプロンプトを送るので、1回の呼び出しが安く、応答も速くなります（中央値1.65秒で、これも最速です）。答えにたどり着くまでの呼び出し回数も少なく、節約の大部分はここから生まれます。

| 解決したタスクあたり | OpenHuman（中央値との比較） | 他の6つの中央値 | 他の6つの最良値 |
| --- | --- | --- | --- |
| 所要時間（p50） | **19.8 s**（2.3倍速い） | 46.5 s | 28.4 s (OpenCode) |
| トークン | **167k**（2.6分の1） | 435k | 370k (Codex) |
| コスト | **$0.0077**（-36%） | $0.012 | $0.0077（OpenCode、同値） |
| ピークメモリ | **68 MB**（7.7分の1） | 523 MB | 123 MB (Codex) |
| CPU 時間 | **1.3 s**（8分の1） | 10.4 s | 2.9 s (DeepSeek Harness) |
| システムプロンプト | **4.6k トークン**（-40%） | 7.7k | 6.2k (DeepSeek Harness) |

---

## 利用者向け

Claude Code、Codex、OpenClaw、Hermes を使ったことがあれば、基本の考え方はご存じのはずです。ツールを使うエージェントループ、MCP、スキル、BYOK モデル、メモリです。OpenHuman はそのすべてを、デスクトップアプリ、ターミナルアプリ、ヘッドレスサーバーで提供します。

| 機能 | OpenHuman が提供するもの |
| --- | --- |
| 設定できるメモリ | [ファイル、リポジトリ、フィード、アプリにわたる組み込みメモリ](https://tinyhumans.gitbook.io/openhuman/features/memory)。選んだメモリエンジン上で、毎ターンの前に出典付きで呼び出されます |
| 音声エージェント | 話の途中で割り込める[ライブ音声エージェント](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice)、音声入力、音声での返答 |
| コンピューターとブラウザの操作 | [実際の Chrome ブラウザとデスクトップアプリを操作](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer)します。元に戻せない操作の前には確認します |
| 検索 | [検索エンジンと検索エージェント](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search): Exa と Gemini は標準搭載、Brave、Tavily、Parallel などは自分のキーで利用できます。出典付きの根拠ある回答と Deep Research にも対応 |
| ツール | [ネイティブツール](https://tinyhumans.gitbook.io/openhuman/features/native-tools): シェルとコーダー、スクレイパー、ドキュメント、画像・動画の生成、cron |
| MCP とスキル | [MCP サーバーとスキルバンドル](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| OAuth 連携 | [Composio 経由の119個のアプリ](https://tinyhumans.gitbook.io/openhuman/features/integrations)。何かが起きたときにエージェントを起動する[トリガー](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers)に対応 |
| モデル | [ローカルモデル（Ollama、LM Studio、MLX）と26プロバイダーの BYOK](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models)、さらに[自動モデルルーティング](https://tinyhumans.gitbook.io/openhuman/features/model-routing) |
| チャネル | エージェントの窓口となる[14のメッセージングチャネル](https://tinyhumans.gitbook.io/openhuman/features/channels): Telegram、Discord、iMessage、メールなど |
| ワークフロー | [永続的なワークフローグラフ](https://tinyhumans.gitbook.io/openhuman/features/workflows): cron、イベント、手動のトリガー、承認ステップ、一時停止後の再開 |
| 安全性 | [承認ゲート](https://tinyhumans.gitbook.io/openhuman/features/approval-gate)、[サンドボックス実行](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security)（OS のジェイルまたは Docker）、ローカルのみで動かす[プライバシーモード](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode)、秘密情報は [OS のキーリング](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage)に保存 |
| 使用量の追跡 | [呼び出しごとのコストとトークン使用量](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage)、再生できる実行ジャーナル |

まずは[はじめに](https://tinyhumans.gitbook.io/openhuman/overview/getting-started)か[ガイド](https://tinyhumans.gitbook.io/openhuman/guides)をご覧ください。

---

## 開発者向け

OpenHuman はライブラリ優先のハーネスです。Rust プロジェクトに追加するだけで、エージェントを普通の関数のように呼び出せます。サイドカーやデーモンを動かす必要はありません。1つのランタイムに数百のエージェントを持たせることができ、それぞれが独自のモデル、ツール、メモリ、サンドボックスを持ちます。月10ドルの VPS に500のエージェントが収まるので、顧客ごとに1エージェントを割り当てるプロダクトも、クラスターなしで始められます。

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

次は、[Rust クイックスタート](https://tinyhumans.gitbook.io/openhuman/developing/quickstart)、[組み込みガイド](../gitbooks/developing/embed/README.md)、[開発者向けドキュメント](https://tinyhumans.gitbook.io/openhuman/developing)をご覧ください。

---

## 他との比較

多くの人がすでに使っているハーネスと OpenHuman を並べてみます。上の4行は[公開ベンチマーク](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md)の結果で、10個の SWE-bench コーディングタスクを、どのハーネスも同じモデルと同じコンテナで実行したものです。数値はすべて解決したタスクあたりです。残りは機能の比較です。

要するに、他のツールは1人が1つのエージェントと作業する用途にとても向いています。OpenHuman もそれに対応し、さらにライブラリとして組み込み、エージェント群へ拡大できるオープンソースの選択肢でもあります。製品は変化が速いので、決める前に各プロジェクトを確認してください。

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **タスク所要時間（中央値）**    | 37.8 s               | 36.1 s               | 62 s                 | 58.5 s               | 🚀 19.8 s                                 |
| **タスクあたり平均トークン** | 428k                 | 370k                 | 442k                 | 482k                 | 🚀 167k                                   |
| **タスクあたりピークメモリ** | 231 MB               | 123 MB               | 1.57 GB              | 816 MB               | 🚀 68 MB                                  |
| **タスクあたり平均コスト**   | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **オープンソース**         | 🚫 プロプライエタリ       | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **エージェント群**        | ⚠️ エージェントごとにプロセス | ⚠️ エージェントごとにプロセス | ⚠️ エージェントごとにプロセス | ⚠️ エージェントごとにプロセス | 🚀 月10ドルの VPS に500エージェント               |
| **組み込めるライブラリ**  | ⚠️ CLI の SDK    | ⚠️ CLI の SDK    | 🚫 なし              | ⚠️ Python パッケージ    | 🚀 型付き Rust API                        |
| **メモリ**              | ⚠️ メモリファイル      | ⚠️ メモリファイル      | ⚠️ プラグイン頼み    | ✅ 自己学習     | 🚀 毎ターン出典付きで呼び出し   |
| **連携**        | ✅ MCP               | ✅ MCP               | ⚠️ 自前で用意               | ⚠️ 自前で用意               | 🚀 119個の OAuth アプリ、MCP、スキル           |
| **メッセージングチャネル**  | 🚫 なし              | 🚫 なし              | ✅ 多数              | ✅ いくつか           | ✅ 14（メール含む）                   |
| **ブラウザとデスクトップ** | ⚠️ MCP 経由のブラウザ   | ⚠️ MCP 経由のブラウザ   | ✅ ブラウザ           | ✅ ブラウザ           | ✅ ブラウザとデスクトップアプリ              |
| **ワークフロー**           | 🚫 なし              | 🚫 なし              | ⚠️ スクリプト           | ⚠️ スクリプト           | 🚀 ビジュアル、エージェントが下書き                 |
| **モデルの選択**        | ⚠️ Anthropic のモデル  | ⚠️ OpenAI 優先      | ✅ 任意               | ✅ 任意               | ✅ 任意、ルーティング内蔵            |

---

## コントリビュート

[`CONTRIBUTING.md`](../CONTRIBUTING.md) をお読みください。または、[このプロンプト](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you)を使って AI コーディングエージェントに案内してもらうこともできます。

1. Git、Node.js 24+、pnpm 10.10.0、Rust 1.96.1（`rustfmt` と `clippy` を含む）、CMake、Ninja、ripgrep、そしてお使いのプラットフォームのデスクトップビルドに必要なものをインストールします。
2. リポジトリをフォークしてクローンします。`git submodule update --init --recursive` を実行し、続けて `pnpm install` を実行します。
3. UI の作業には `pnpm dev`、デスクトップアプリには `pnpm dev:app` を実行します。PR を開く前に、`pnpm typecheck`、`pnpm format:check`、`cargo check --manifest-path Cargo.toml` を実行してください。

詳しくは[環境構築](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up)、[`AGENTS.md`](../AGENTS.md)、[crates の概要](../crates/README.md)をご覧ください。OpenHuman の多くの部分は [`vendor/`](../vendor) 配下の独立したリポジトリにあり、そちらへの貢献も歓迎します。

コントリビューターには、無料のグッズと [Discord](https://guild.tinyhumans.ai/) での特別なアクセスを提供します。

## スター履歴

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="スター履歴のチャート" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

## コントリビューター

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman のコントリビューター" />
</a>
