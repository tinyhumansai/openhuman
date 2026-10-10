<h1 align="center">OpenHuman</h1>

<p align="center">
 <strong>Das schnellste, günstigste und effizienteste Open-Source-Agent-Harness. Betreibe mehr als 500 Agenten auf einem VPS für 10 $.</strong><br/>
 Eine Desktop-App für Menschen. Eine Rust-Bibliothek für Entwickler.
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Frühe Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Aktuellste Version" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="Lizenz" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="Öffentliche Benchmarks" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">Dokumentation</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust-Schnellstart</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">Benchmarks</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">Diskussionen</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel folgen (Creator)</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="Ein Rundgang durch die OpenHuman-Desktop-App" />
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
> 🎉 Innerhalb einer Woche nach dem Start wurde OpenHuman neun Tage in Folge zum **Repository auf Platz 1 der GitHub-Trends**.

> **Frühe Beta.** OpenHuman wird aktiv entwickelt, rechne daher mit Ecken und Kanten.

---

## Installation

Am einfachsten lädst du die Desktop-App von [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) oder aus dem [neuesten Release](https://github.com/tinyhumansai/openhuman/releases/latest) herunter. Es gibt eine `.dmg` für macOS, eine `.msi` oder `.exe` für Windows und eine `.deb` oder `.AppImage` für Linux.

Lieber im Terminal? Das Installationsskript wählt das passende Paket für dein System, prüft die Prüfsumme und installiert es:

```bash
# macOS und Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

Um vorab zu sehen, was das Skript für macOS und Linux tun würde, hänge `bash -s -- --dry-run` an den Befehl an. Weitere Optionen und Hilfe bei Problemen findest du in [INSTALL.md](../INSTALL.md).

---

## Warum OpenHuman?

Die meisten Agent-Harnesses starten pro Agent einen schweren Prozess und senden bei jedem Aufruf einen großen Prompt erneut. OpenHuman erledigt dieselbe Arbeit mit weit weniger. Es ist das einzige funktionsreiche Open-Source-Harness, das für große Agentenflotten gebaut ist: 500 davon passen auf einen Server für 10 $.

<table>

<tr>

<td width="50%" valign="top">

<h3>Schnell</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">Benchmark-Ergebnisse</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">Zahlen zum Kaltstart</a></p>

<p>Erledigt Programmieraufgaben in etwa 20 Sekunden und ist damit das schnellste von sieben getesteten KI-Agent-Tools. Startet in einer Zehntelsekunde.</p>

</td>

<td width="50%" valign="top">

<h3>Günstig</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">Kosten- und Token-Daten</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">So funktioniert die Kompression</a></p>

<p>Braucht 2,6x weniger Tokens als das typische Agent-Tool und hatte in unserem Test die niedrigste Gesamtrechnung.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>Effizient im großen Maßstab</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">Messungen mit Agentenflotten</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">Methodik</a></p>

<p>Braucht etwa 8x weniger Arbeitsspeicher und CPU als das typische Agent-Tool. Betreibe mehr als 500 Agenten auf einem Server für 10 $.</p>

</td>

<td width="50%" valign="top">

<h3>Für Entwickler gebaut</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust-Schnellstart</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/embed">Einbettungsanleitung</a> · <a href="../crates/openhuman-embed/examples">Beispiele</a></p>

<p>Nutze es als Rust-Bibliothek: Rufe einen Agenten wie jede andere Funktion auf oder betreibe eine ganze Flotte auf einem kleinen Server.</p>

</td>

</tr>

</table>


<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes gegen OpenHuman" />
</p>

<p align="center"><em>Derselbe Prompt, nebeneinander: „Schreib mir eine Geschichte über Agent-Harnesses in Form einer Anime-Geschichte, schreibe sie in eine HTML-Seite und öffne sie für mich.“<br/>OpenHuman war in 20 s fertig und brauchte 15k Tokens für $0.0054. Hermes brauchte 9 min 40 s und 37k Tokens für $0.0082.</em></p>

---

## Die wichtigsten Neuerungen

Die meisten Agent-Harnesses sind eine einfache Schleife: alles ans Modell senden, warten, wiederholen. Das funktioniert für einen Agenten, wird aber schnell langsam und teuer und bricht zusammen, sobald du Hunderte betreibst.

OpenHuman denkt die teuersten Teile neu: wie viel Text die KI lesen muss, wie sie das richtige Werkzeug findet, wie Funktionen geladen werden und wie viel Rechner das Ganze braucht. Die sechs Ideen unten sind der Grund für die Geschwindigkeit und die Einsparungen von oben. Jede Karte verlinkt auf die Dokumentation, falls du Details willst.

<table>

<tr>

<td width="50%" valign="top">

<h3>RLM-Token-Kompression</h3>

<p><a href="https://arxiv.org/abs/2512.24601">RLM-Paper</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">So funktioniert es</a></p>

<p>Basiert auf <a href="https://arxiv.org/abs/2512.24601">Recursive Language Models</a>. Große Werkzeugergebnisse werden komprimiert, bevor die KI sie liest. Bei sehr großen bekommt die KI ein Handle, das sie durchsuchen kann, statt alles zu lesen. Nichts wird weggeworfen.</p>

</td>

<td width="50%" valign="top">

<h3>Jev: sofortige, genaue Werkzeugsuche</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">Die Messungen</a></p>

<p>Jev ist ein winziges Modell, das aus 1.215 Werkzeugen das richtige findet. Das richtige ist in 86,8 % der Fälle unter seinen Top-Treffern, bei der Stichwortsuche nur in 70,5 %.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>Einheitlicher Rust-Bus</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">So funktionieren Plug-ins</a></p>

<p>Jede Funktion, etwa Suche, Dokumente oder Sprache, hängt an einem gemeinsamen Rust-Bus, eine Idee vom <a href="https://www.freedesktop.org/wiki/Software/dbus/">Linux-Systembus</a>. Eine Funktion wird nur bei Bedarf geladen, und wenn eine hängt, arbeiten die anderen weiter.</p>

</td>

<td width="50%" valign="top">

<h3>Tief integriertes Gedächtnis</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">So funktioniert das Gedächtnis</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">Gedächtnis-Engines</a></p>

<p>Das Gedächtnis ist eingebaut. Vor jedem Zug wählt OpenHuman nur das Wichtige aus, innerhalb eines Token-Budgets, und übergibt es der KI mit Quellenangaben. Es funktioniert sofort, und mit einer Einstellung tauschst du die Gedächtnis-Engine aus.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>Sofortige Browser- und Desktop-Steuerung</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">Browser- und Computersteuerung</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>Der Agent nutzt einen echten Browser und deine Desktop-Apps. Jev wählt jeden Klick aus den Schaltflächen auf dem Bildschirm, ganz ohne Screenshots. Vor jeder Zahlung hält er an.</p>

</td>

<td width="50%" valign="top">

<h3>Ein programmierbarer Rust-Kern</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust quickstart</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">Leistung</a></p>

<p>Das gesamte Harness ist kompiliertes Rust in einem einzigen Prozess. Es startet in einer Zehntelsekunde und bleibt schlank: 68 MB im Spitzenwert bei unseren Programmieraufgaben. Derselbe Kern ist eine Bibliothek. Rufe einen Agenten aus deinem eigenen Rust-Code auf oder betreibe Hunderte nebeneinander auf einem kleinen Server.</p>

</td>

</tr>

</table>

---

## Benchmarks

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench-Verified-Lauf swe-x86-1: OpenHuman gegen sechs andere Harnesses in zehn Diagrammen" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

Wir haben sieben Harnesses auf denselben SWE-bench-Aufgaben laufen lassen, mit demselben Modell, demselben API-Schlüssel und demselben Container, und jeden Aufruf gemessen. Aufbau und Ergebnisse sind in [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) öffentlich, sodass jeder sie wiederholen kann. Der neueste Lauf ist [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md) mit zehn Aufgaben.

OpenHuman erledigte seine Aufgaben in weniger als der halben Median-Zeit, mit 2,6x weniger Tokens und einem Achtel des Arbeitsspeichers und der CPU. Es machte 114 Modellaufrufe, die anderen 134 bis 212, und sein gesamter Lauf kostete $0.05, die der anderen $0.08 bis $0.35.

Es gewinnt, weil es pro Schritt weniger Arbeit leistet. Der Kern ist kompiliertes Rust in einem einzigen Prozess, nicht Node oder Python, daher braucht er wenig Arbeitsspeicher und zwischen den Modellaufrufen fast keine CPU. Es sendet den kleinsten Prompt der sieben, 4,6k Tokens einschließlich seiner 20 Werkzeuge, sodass jeder Aufruf günstiger ist und schneller zurückkommt (1,65 s im Median, ebenfalls der schnellste Wert). Außerdem braucht es weniger Aufrufe bis zur Antwort, und daher kommt der Großteil der Einsparungen.

| Pro gelöster Aufgabe | OpenHuman (im Vergleich zum Median) | Median der anderen sechs | Bester der anderen sechs |
| --- | --- | --- | --- |
| Laufzeit (p50) | **19.8 s** (2,3x schneller) | 46.5 s | 28.4 s (OpenCode) |
| Tokens | **167k** (2,6x weniger) | 435k | 370k (Codex) |
| Kosten | **$0.0077** (-36 %) | $0.012 | $0.0077 (OpenCode, Gleichstand) |
| Spitzen-Arbeitsspeicher | **68 MB** (7,7x weniger) | 523 MB | 123 MB (Codex) |
| CPU-Zeit | **1.3 s** (8x weniger) | 10.4 s | 2.9 s (DeepSeek Harness) |
| System-Prompt | **4.6k Tokens** (-40 %) | 7.7k | 6.2k (DeepSeek Harness) |

---

## Für Nutzer

Wenn du Claude Code, Codex, OpenClaw oder Hermes nutzt, kennst du die Ideen schon: eine Agentenschleife mit Werkzeugen, MCP, Skills, BYOK-Modelle und Gedächtnis. OpenHuman bietet alles davon, in einer Desktop-App, einer Terminal-App oder auf einem Headless-Server.

| Fähigkeit | Was OpenHuman mitbringt |
| --- | --- |
| Konfigurierbares Gedächtnis | [Eingebautes Gedächtnis über deine Dateien, Repos, Feeds und Apps](https://tinyhumans.gitbook.io/openhuman/features/memory), vor jedem Zug mit Quellenangaben abgerufen, auf der Gedächtnis-Engine deiner Wahl |
| Sprach-Agenten | Ein [Live-Sprach-Agent](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice) den du mitten im Satz unterbrechen kannst, dazu Diktat und gesprochene Antworten |
| Computer- und Browsersteuerung | [Steuert einen echten Chrome-Browser und deine Desktop-Apps](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer) und fragt nach, bevor etwas geschieht, das sich nicht rückgängig machen lässt |
| Suche | [Suchmaschinen und Such-Agenten](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search): Exa und Gemini inklusive, Brave, Tavily, Parallel und mehr mit deinem eigenen Schlüssel, fundierte Antworten mit Quellenangaben und Deep Research |
| Werkzeuge | [Native Werkzeuge](https://tinyhumans.gitbook.io/openhuman/features/native-tools): Shell und Coder, Scraper, Dokumente, Bild- und Videogenerierung, Cron |
| MCP und Skills | [MCP-Server und Skill-Pakete](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| OAuth-Integrationen | [119 Apps über Composio](https://tinyhumans.gitbook.io/openhuman/features/integrations), mit [Triggern](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers), die den Agenten starten, wenn etwas passiert |
| Modelle | [Lokale Modelle (Ollama, LM Studio, MLX) und BYOK für 26 Anbieter](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models), mit [automatischer Modellauswahl](https://tinyhumans.gitbook.io/openhuman/features/model-routing) |
| Kanäle | [14 Messaging-Kanäle](https://tinyhumans.gitbook.io/openhuman/features/channels) als Oberflächen für den Agenten: Telegram, Discord, iMessage, E-Mail und mehr |
| Workflows | [Dauerhafte Workflow-Graphen](https://tinyhumans.gitbook.io/openhuman/features/workflows): Cron-, Ereignis- oder manuelle Trigger, Freigabeschritte, Fortsetzen nach einer Pause |
| Sicherheit | [Freigabeschranke](https://tinyhumans.gitbook.io/openhuman/features/approval-gate), [Ausführung in einer Sandbox](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security) (OS-Jail oder Docker), [Privacy Mode](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode) für rein lokale Läufe, Geheimnisse im [OS-Schlüsselbund](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage) |
| Nutzungserfassung | [Kosten und Token-Verbrauch pro Aufruf](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage), dazu wiederabspielbare Lauf-Protokolle |

Fang mit dem [Einstieg](https://tinyhumans.gitbook.io/openhuman/overview/getting-started) oder den [Anleitungen](https://tinyhumans.gitbook.io/openhuman/guides) an.

---

## Für Entwickler

OpenHuman ist ein Harness, das zuerst als Bibliothek gedacht ist. Füge es deinem Rust-Projekt hinzu und rufe einen Agenten wie jede andere Funktion auf, ohne Sidecar oder Daemon. Eine Runtime kann Hunderte Agenten halten, jeden mit eigenem Modell, eigenen Werkzeugen, eigenem Gedächtnis und eigener Sandbox. 500 davon passen auf einen VPS für 10 $, sodass ein Produkt mit einem Agenten pro Kunde ohne Cluster starten kann.

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

Als Nächstes: der [Rust-Schnellstart](https://tinyhumans.gitbook.io/openhuman/developing/quickstart), die [Einbettungsanleitung](https://tinyhumans.gitbook.io/openhuman/developing/embed) und die [Entwicklerdokumentation](https://tinyhumans.gitbook.io/openhuman/developing).

---

## Wie schneidet es im Vergleich ab?

Hier steht OpenHuman neben den Harnesses, die die meisten schon nutzen. Die oberen vier Zeilen stammen aus unserem [öffentlichen Benchmark](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md): zehn SWE-bench-Programmieraufgaben, mit demselben Modell und Container für jedes Harness. Jede Zahl gilt pro gelöster Aufgabe. Der Rest vergleicht Funktionen.

Kurz gesagt: Die anderen sind großartig für eine Person, die mit einem Agenten arbeitet. OpenHuman kann das auch, und es ist zusätzlich die Open-Source-Option, die du als Bibliothek einbetten und zu einer Flotte skalieren kannst. Produkte ändern sich schnell, prüfe daher jedes Projekt, bevor du dich entscheidest.

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **Mediane Aufgabenzeit** | 37.8 s               | 36.1 s               | 62 s                 | 58.5 s               | 🚀 19.8 s                                 |
| **Ø Tokens pro Aufgabe** | 428k                 | 370k                 | 442k                 | 482k                 | 🚀 167k                                   |
| **Spitzen-RAM pro Aufgabe** | 231 MB               | 123 MB               | 1.57 GB              | 816 MB               | 🚀 68 MB                                  |
| **Ø Kosten pro Aufgabe** | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **Open Source**         | 🚫 Proprietär       | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **Agentenflotten**      | ⚠️ Prozess pro Agent | ⚠️ Prozess pro Agent | ⚠️ Prozess pro Agent | ⚠️ Prozess pro Agent | 🚀 500 Agenten auf einem VPS für 10 $               |
| **Einbettbare Bibliothek** | ⚠️ SDK über eine CLI    | ⚠️ SDK über eine CLI    | 🚫 Keine              | ⚠️ Python-Paket    | 🚀 Typisierte Rust-API                        |
| **Gedächtnis**          | ⚠️ Gedächtnis-Dateien      | ⚠️ Gedächtnis-Dateien      | ⚠️ Auf Plug-ins angewiesen    | ✅ Selbstlernend     | 🚀 Bei jedem Zug abgerufen, mit Quellenangaben   |
| **Integrationen**       | ✅ MCP               | ✅ MCP               | ⚠️ Selbst mitbringen               | ⚠️ Selbst mitbringen               | 🚀 119 OAuth-Apps, MCP, Skills           |
| **Messaging-Kanäle**    | 🚫 Keine              | 🚫 Keine              | ✅ Viele              | ✅ Mehrere           | ✅ 14, darunter E-Mail                   |
| **Browser und Desktop** | ⚠️ Browser über MCP   | ⚠️ Browser über MCP   | ✅ Browser           | ✅ Browser           | ✅ Browser und Desktop-Apps              |
| **Workflows**           | 🚫 Keine              | 🚫 Keine              | ⚠️ Skripte           | ⚠️ Skripte           | 🚀 Visuell, vom Agenten entworfen                 |
| **Modellwahl**          | ⚠️ Anthropic-Modelle  | ⚠️ OpenAI zuerst      | ✅ Beliebig               | ✅ Beliebig               | ✅ Beliebig, mit eingebauter Modellauswahl            |

---

## Mitmachen

Lies [`CONTRIBUTING.md`](../CONTRIBUTING.md) oder lass dich von einem KI-Coding-Agenten mit [diesem Prompt](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you) anleiten.

1. Installiere Git, Node.js 24+, pnpm 10.10.0, Rust 1.96.1 (mit `rustfmt` und `clippy`), CMake, Ninja, ripgrep und die Voraussetzungen für den Desktop-Build deiner Plattform.
2. Forke das Repo und klone es. Führe `git submodule update --init --recursive` aus, dann `pnpm install`.
3. Starte `pnpm dev` für Arbeit an der Oberfläche oder `pnpm dev:app` für die Desktop-App. Bevor du einen PR öffnest, führe `pnpm typecheck`, `pnpm format:check` und `cargo check --manifest-path Cargo.toml` aus.

Mehr dazu in [Einrichtung](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up), [`AGENTS.md`](../AGENTS.md) und der [Crates-Übersicht](../crates/README.md). Viele Teile von OpenHuman liegen in eigenen Repos unter [`vendor/`](../vendor), und auch dort sind Beiträge willkommen.

Mitwirkende bekommen kostenlosen Merch und Sonderzugang auf [Discord](https://guild.tinyhumans.ai/).

## Star-Verlauf

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="Star-Verlauf" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

## Mitwirkende

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman-Mitwirkende" />
</a>
