<h1 align="center">OpenHuman</h1>

<p align="center">
 <strong>가장 빠르고, 가장 저렴하고, 가장 효율적인 오픈소스 에이전트 하네스. $10짜리 VPS 한 대에서 500개 이상의 에이전트를 실행하세요.</strong><br/>
 사람을 위한 데스크톱 앱. 개발자를 위한 Rust 라이브러리.
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="얼리 베타" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="최신 릴리스" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub 스타" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="라이선스" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="공개 벤치마크" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">문서</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust 퀵스타트</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">벤치마크</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">토론</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel (제작자)</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="OpenHuman 데스크톱 앱 둘러보기" />
</p>

<p align="center">
	<a href="https://trendshift.io/repositories/23680" target="_blank">
		<img src="https://trendshift.io/api/badge/repositories/23680" alt="tinyhumansai%2Fopenhuman | Trendshift" width="250" height="55"/>
	</a>
	<a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
		<img alt="Product Hunt의 OpenHuman" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=daily&amp;t=1778916022823">
	</a>
	<a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
		<img alt="Product Hunt의 OpenHuman" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=weekly&amp;t=1779351403565">
	</a>
</p>

<p align="center">
  🇺🇸 <a href="../README.md">English</a> | 🇸🇦 <a href="./README.ar.md">العربية</a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md"><strong>한국어</strong></a> | 🇩🇪 <a href="./README.de.md">Deutsch</a> | 🇹🇷 <a href="./README.tr.md">Türkçe</a> | 🇵🇰 <a href="./README.ur-pk.md">اردو</a>
</p>

> [!NOTE]
> 🎉 출시 일주일 만에 OpenHuman은 9일 연속 **GitHub 트렌딩 1위 저장소**가 되었습니다.

> **얼리 베타.** OpenHuman은 활발히 개발 중이므로 거친 부분이 있을 수 있습니다.

---

## 설치

가장 쉬운 방법은 [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) 또는 [최신 릴리스](https://github.com/tinyhumansai/openhuman/releases/latest)에서 데스크톱 앱을 내려받는 것입니다. macOS용 `.dmg`, Windows용 `.msi` 또는 `.exe`, Linux용 `.deb` 또는 `.AppImage`가 있습니다.

터미널이 편하신가요? 설치 스크립트가 시스템에 맞는 패키지를 골라 체크섬을 확인한 뒤 설치합니다.

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

macOS와 Linux 스크립트가 무엇을 하는지 미리 보려면 명령 끝에 `bash -s -- --dry-run`을 붙이세요. 다른 옵션과 문제 해결 방법은 [INSTALL.md](../INSTALL.md)에 있습니다.

---

## 왜 OpenHuman인가요?

대부분의 에이전트 하네스는 에이전트마다 무거운 프로세스를 하나씩 띄우고, 호출할 때마다 큰 프롬프트를 다시 보냅니다. OpenHuman은 같은 일을 훨씬 적은 자원으로 해냅니다. 대규모 에이전트 실행을 위해 만들어진, 기능이 풍부한 유일한 오픈소스 하네스이며, $10짜리 서버 한 대에 500개가 들어갑니다.

<table>

<tr>

<td width="50%" valign="top">

<h3>빠릅니다</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">벤치마크 결과</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">콜드 스타트 수치</a></p>

<p>코딩 작업을 약 20초 만에 끝냅니다. 테스트한 AI 에이전트 도구 7개 중 가장 빠릅니다. 시작하는 데는 0.1초가 걸립니다.</p>

</td>

<td width="50%" valign="top">

<h3>저렴합니다</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">비용 및 토큰 데이터</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">압축 방식</a></p>

<p>일반적인 에이전트 도구보다 토큰을 2.6배 적게 쓰며, 테스트에서 전체 비용이 가장 낮았습니다.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>대규모에서도 효율적입니다</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">대규모 실행 측정 결과</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">측정 방법</a></p>

<p>일반적인 에이전트 도구보다 메모리와 CPU를 약 8배 적게 씁니다. $10짜리 서버 한 대에서 500개 이상의 에이전트를 실행하세요.</p>

</td>

<td width="50%" valign="top">

<h3>개발자를 위해 만들었습니다</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust 퀵스타트</a> · <a href="../gitbooks/developing/embed/README.md">임베딩 가이드</a> · <a href="../crates/openhuman-embed/examples">예제</a></p>

<p>Rust 라이브러리로 사용하세요. 다른 함수처럼 에이전트를 호출하거나, 작은 서버 하나에서 수많은 에이전트를 실행할 수 있습니다.</p>

</td>

</tr>

</table>


<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes와 OpenHuman 비교" />
</p>

<p align="center"><em>같은 프롬프트를 나란히 비교: "에이전트 하네스에 관한 이야기를 애니메이션 스토리 형식으로 써서 HTML 페이지로 만들고 열어 줘."<br/>OpenHuman은 20초 만에 끝냈고, 토큰 15k개와 $0.0054를 썼습니다. Hermes는 9분 40초가 걸렸고, 토큰 37k개와 $0.0082를 썼습니다.</em></p>

---

## 주요 혁신

대부분의 에이전트 하네스는 단순한 반복입니다. 모든 것을 모델에 보내고, 기다리고, 다시 반복합니다. 에이전트 하나라면 괜찮지만, 금세 느려지고 비싸지며, 수백 개를 돌리면 무너집니다.

OpenHuman은 비용이 가장 많이 드는 부분을 다시 생각했습니다. AI가 읽어야 하는 글의 양, 알맞은 도구를 찾는 방법, 기능을 불러오는 방식, 전체가 필요로 하는 컴퓨터 자원의 크기입니다. 아래 여섯 가지 아이디어가 위에서 본 속도와 절감의 원천입니다. 각 카드는 자세한 내용이 담긴 문서로 연결됩니다.

<table>

<tr>

<td width="50%" valign="top">

<h3>RLM 토큰 압축</h3>

<p><a href="https://arxiv.org/abs/2512.24601">RLM 논문</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">작동 방식</a></p>

<p><a href="https://arxiv.org/abs/2512.24601">Recursive Language Models</a>를 바탕으로 합니다. 큰 도구 결과는 AI가 읽기 전에 압축됩니다. 아주 큰 결과는 AI가 전부 읽는 대신 검색할 수 있는 핸들을 받습니다. 버려지는 것은 없습니다.</p>

</td>

<td width="50%" valign="top">

<h3>Jev: 즉각적이고 정확한 도구 검색</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">측정 결과</a></p>

<p>Jev는 1,215개의 도구 중에서 알맞은 도구를 찾아내는 작은 모델입니다. 알맞은 도구가 상위 후보에 들어 있는 비율은 86.8%로, 키워드 검색의 70.5%보다 높습니다.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>통합 Rust 버스</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">플러그인 작동 방식</a></p>

<p>검색, 문서, 음성 같은 모든 기능이 하나의 Rust 버스에 연결됩니다. <a href="https://packages.debian.org/stable/dbus">Linux 시스템 버스</a>에서 빌려 온 아이디어입니다. 기능은 필요할 때만 로드되고, 하나가 멈춰도 나머지는 계속 작동합니다.</p>

</td>

<td width="50%" valign="top">

<h3>깊이 통합된 메모리</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">메모리 작동 방식</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">메모리 엔진</a></p>

<p>메모리가 기본으로 들어 있습니다. 매 턴 전에 OpenHuman은 토큰 예산 안에서 중요한 것만 골라 출처와 함께 AI에 전달합니다. 별도 설정 없이 바로 작동하며, 설정 하나로 다른 메모리 엔진으로 바꿀 수도 있습니다.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>즉각적인 브라우저 및 데스크톱 제어</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">브라우저 및 컴퓨터 제어</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>에이전트는 실제 브라우저와 데스크톱 앱을 사용합니다. Jev는 화면에 보이는 버튼 중에서 클릭할 곳을 고르며, 스크린샷은 쓰지 않습니다. 결제 직전에는 멈춥니다.</p>

</td>

<td width="50%" valign="top">

<h3>프로그래밍 가능한 Rust 코어</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust 퀵스타트</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">성능</a></p>

<p>하네스 전체가 하나의 프로세스에서 도는 컴파일된 Rust라서, 0.1초 만에 시작하고 가볍게 유지됩니다. 코딩 작업에서 최대 68 MB였습니다. 같은 코어가 라이브러리이기도 합니다. 직접 작성한 Rust 코드에서 에이전트를 호출하거나, 작은 서버 하나에서 수백 개를 나란히 실행하세요.</p>

</td>

</tr>

</table>

---

## 벤치마크

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench Verified 실행 swe-x86-1: OpenHuman과 다른 하네스 6개를 패널 10개로 비교" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

같은 SWE-bench 작업을 같은 모델, API 키, 컨테이너로 하네스 7개에서 실행하고 모든 호출을 측정했습니다. 설정과 결과는 [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks)에 공개되어 있어 누구나 다시 실행할 수 있습니다. 가장 최근 실행은 작업 10개로 진행한 [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md)입니다.

OpenHuman은 중앙값 시간의 절반도 안 되는 시간에 작업을 끝냈고, 토큰은 2.6배 적게, 메모리와 CPU는 8분의 1만 썼습니다. 모델 호출은 114번이었고 다른 하네스는 134번에서 212번이었습니다. 전체 실행 비용은 $0.05였고 다른 하네스는 $0.08에서 $0.35였습니다.

단계마다 일을 덜 하기 때문에 이깁니다. 코어는 Node나 Python이 아니라 단일 프로세스의 컴파일된 Rust여서, 메모리를 적게 쓰고 모델 호출 사이에는 CPU를 거의 쓰지 않습니다. 도구 20개를 포함해 4.6k 토큰으로, 7개 중 가장 작은 프롬프트를 보냅니다. 그래서 호출마다 더 저렴하고 더 빨리 돌아옵니다(중앙값 1.65초, 이 역시 가장 빠릅니다). 답에 이르기까지 필요한 호출 수도 적은데, 절감 효과의 대부분이 여기서 나옵니다.

| 해결한 작업당 | OpenHuman (중앙값 대비) | 다른 6개의 중앙값 | 다른 6개 중 최고 |
| --- | --- | --- | --- |
| 소요 시간 (p50) | **19.8 s** (2.3배 빠름) | 46.5 s | 28.4 s (OpenCode) |
| 토큰 | **167k** (2.6배 적음) | 435k | 370k (Codex) |
| 비용 | **$0.0077** (-36%) | $0.012 | $0.0077 (OpenCode, 동률) |
| 최대 메모리 | **68 MB** (7.7배 적음) | 523 MB | 123 MB (Codex) |
| CPU 시간 | **1.3 s** (8배 적음) | 10.4 s | 2.9 s (DeepSeek Harness) |
| 시스템 프롬프트 | **4.6k 토큰** (-40%) | 7.7k | 6.2k (DeepSeek Harness) |

---

## 사용자를 위해

Claude Code, Codex, OpenClaw, Hermes를 써 보셨다면 이미 아는 개념들입니다. 도구를 쓰는 에이전트 루프, MCP, 스킬, BYOK 모델, 메모리입니다. OpenHuman은 이 모두를 데스크톱 앱, 터미널 앱, 헤드리스 서버에서 제공합니다.

| 기능 | OpenHuman이 제공하는 것 |
| --- | --- |
| 설정 가능한 메모리 | [파일, 저장소, 피드, 앱에 대한 내장 메모리](https://tinyhumans.gitbook.io/openhuman/features/memory). 선택한 메모리 엔진 위에서 매 턴 전에 출처와 함께 불러옵니다 |
| 음성 에이전트 | 말하는 도중에 끼어들 수 있는 [실시간 음성 에이전트](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice), 그리고 받아쓰기와 음성 답변 |
| 컴퓨터 및 브라우저 제어 | [실제 Chrome 브라우저와 데스크톱 앱을 조작](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer)하며, 되돌릴 수 없는 일은 먼저 물어봅니다 |
| 검색 | [검색 엔진과 검색 에이전트](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search): Exa와 Gemini 포함, 본인의 키로 Brave, Tavily, Parallel 등 사용 가능, 출처가 있는 근거 기반 답변과 Deep Research |
| 도구 | [네이티브 도구](https://tinyhumans.gitbook.io/openhuman/features/native-tools): 셸과 코더, 스크레이퍼, 문서, 이미지 및 영상 생성, cron |
| MCP와 스킬 | [MCP 서버와 스킬 번들](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| OAuth 연동 | [Composio를 통한 119개 앱](https://tinyhumans.gitbook.io/openhuman/features/integrations), 무슨 일이 생기면 에이전트를 시작하는 [트리거](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers) 포함 |
| 모델 | [로컬 모델(Ollama, LM Studio, MLX)과 26개 제공업체용 BYOK](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models), 그리고 [자동 모델 라우팅](https://tinyhumans.gitbook.io/openhuman/features/model-routing) |
| 채널 | 에이전트 프런트엔드로 쓰는 [메시징 채널 14개](https://tinyhumans.gitbook.io/openhuman/features/channels): Telegram, Discord, iMessage, 이메일 등 |
| 워크플로 | [내구성 있는 워크플로 그래프](https://tinyhumans.gitbook.io/openhuman/features/workflows): cron, 이벤트, 수동 트리거, 승인 단계, 일시 중지 후 재개 |
| 안전 | [승인 게이트](https://tinyhumans.gitbook.io/openhuman/features/approval-gate), [샌드박스 실행](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security)(OS 격리 또는 Docker), 로컬 전용 실행을 위한 [프라이버시 모드](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode), [OS 키링](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage)에 보관하는 비밀 정보 |
| 사용량 추적 | [호출별 비용과 토큰 사용량](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage), 그리고 다시 재생할 수 있는 실행 기록 |

[시작하기](https://tinyhumans.gitbook.io/openhuman/overview/getting-started) 또는 [가이드](https://tinyhumans.gitbook.io/openhuman/guides)부터 보세요.

---

## 개발자를 위해

OpenHuman은 라이브러리 우선 하네스입니다. Rust 프로젝트에 추가하고, 사이드카나 데몬 없이 다른 함수처럼 에이전트를 호출하세요. 런타임 하나가 수백 개의 에이전트를 담을 수 있고, 각 에이전트는 자기만의 모델, 도구, 메모리, 샌드박스를 가집니다. $10짜리 VPS 한 대에 500개가 들어가므로, 고객마다 에이전트 하나를 두는 제품도 클러스터 없이 시작할 수 있습니다.

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

다음으로는 [Rust 퀵스타트](https://tinyhumans.gitbook.io/openhuman/developing/quickstart), [임베딩 가이드](../gitbooks/developing/embed/README.md), [개발자 문서](https://tinyhumans.gitbook.io/openhuman/developing)를 보세요.

---

## 다른 도구와 비교하면?

사람들이 이미 많이 쓰는 하네스와 OpenHuman을 나란히 놓았습니다. 위쪽 네 행은 [공개 벤치마크](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md)에서 가져왔습니다. SWE-bench 코딩 작업 10개를 모든 하네스에 같은 모델과 컨테이너로 실행했고, 각 수치는 해결한 작업당 값입니다. 나머지는 기능을 비교한 것입니다.

한마디로, 다른 도구들은 한 사람이 에이전트 하나와 일할 때 훌륭합니다. OpenHuman도 그렇게 쓸 수 있고, 라이브러리로 임베딩해 대규모 에이전트로 확장할 수 있는 오픈소스 선택지이기도 합니다. 제품은 빠르게 바뀌므로 결정하기 전에 각 프로젝트를 확인하세요.

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **작업 시간 중앙값**    | 37.8 s               | 36.1 s               | 62 s                 | 58.5 s               | 🚀 19.8 s                                 |
| **작업당 평균 토큰** | 428k                 | 370k                 | 442k                 | 482k                 | 🚀 167k                                   |
| **작업당 최대 메모리** | 231 MB               | 123 MB               | 1.57 GB              | 816 MB               | 🚀 68 MB                                  |
| **작업당 평균 비용**   | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **오픈소스**         | 🚫 독점       | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **에이전트 무리**        | ⚠️ 에이전트마다 프로세스 | ⚠️ 에이전트마다 프로세스 | ⚠️ 에이전트마다 프로세스 | ⚠️ 에이전트마다 프로세스 | 🚀 $10 VPS에서 에이전트 500개               |
| **임베딩 가능한 라이브러리**  | ⚠️ CLI 위의 SDK    | ⚠️ CLI 위의 SDK    | 🚫 없음              | ⚠️ Python 패키지    | 🚀 타입이 있는 Rust API                        |
| **메모리**              | ⚠️ 메모리 파일      | ⚠️ 메모리 파일      | ⚠️ 플러그인 의존    | ✅ 자기 학습     | 🚀 매 턴 출처와 함께 불러옴   |
| **연동**        | ✅ MCP               | ✅ MCP               | ⚠️ 직접 구성               | ⚠️ 직접 구성               | 🚀 OAuth 앱 119개, MCP, 스킬           |
| **메시징 채널**  | 🚫 없음              | 🚫 없음              | ✅ 다수              | ✅ 여러 개           | ✅ 이메일 포함 14개                   |
| **브라우저 및 데스크톱** | ⚠️ MCP를 통한 브라우저   | ⚠️ MCP를 통한 브라우저   | ✅ 브라우저           | ✅ 브라우저           | ✅ 브라우저와 데스크톱 앱              |
| **워크플로**           | 🚫 없음              | 🚫 없음              | ⚠️ 스크립트           | ⚠️ 스크립트           | 🚀 시각적, 에이전트가 초안 작성                 |
| **모델 선택**        | ⚠️ Anthropic 모델  | ⚠️ OpenAI 우선      | ✅ 모두               | ✅ 모두               | ✅ 모두, 내장 라우팅 제공            |

---

## 기여하기

[`CONTRIBUTING.md`](../CONTRIBUTING.md)를 읽거나, [이 프롬프트](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you)로 AI 코딩 에이전트의 안내를 받아 보세요.

1. Git, Node.js 24+, pnpm 10.10.0, Rust 1.96.1(`rustfmt`와 `clippy` 포함), CMake, Ninja, ripgrep, 그리고 사용 중인 플랫폼의 데스크톱 빌드 사전 요구 사항을 설치합니다.
2. 저장소를 포크하고 클론합니다. `git submodule update --init --recursive`를 실행한 뒤 `pnpm install`을 실행합니다.
3. UI 작업에는 `pnpm dev`, 데스크톱 앱에는 `pnpm dev:app`을 실행합니다. PR을 열기 전에 `pnpm typecheck`, `pnpm format:check`, `cargo check --manifest-path Cargo.toml`을 실행하세요.

자세한 내용은 [개발 환경 설정](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up), [`AGENTS.md`](../AGENTS.md), [크레이트 개요](../crates/README.md)에 있습니다. OpenHuman의 많은 부분은 [`vendor/`](../vendor) 아래의 별도 저장소에 있으며, 그곳도 기여를 환영합니다.

기여자에게는 무료 굿즈와 [Discord](https://guild.tinyhumans.ai/)의 특별 접근 권한을 드립니다.

## 스타 히스토리

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="스타 히스토리 차트" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

## 기여자

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman 기여자" />
</a>
