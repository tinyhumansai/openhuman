<p align="center">
  🇺🇸 <a href="./README.md">English</a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md">한국어</a> | 🇩🇪 <a href="./README.de.md">Deutsch</a>
</p>

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
 <strong>OpenHuman ist deine persönliche KI-Superintelligenz: Lokaler Speicher, verwaltete Dienste wo nötig, schlicht und mächtig.</strong>
</p>


<p align="center">
 <a href="https://discord.tinyhumans.ai/">Discord</a> •
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> •
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X/Twitter</a> •
 <a href="https://tinyhumans.gitbook.io/openhuman/">Doku</a> •
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel folgen (Creator)</a>
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Frühe Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Aktuellste Version" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="./LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="Lizenz" /></a>
 <a href="./README.md"><img src="https://img.shields.io/badge/lang-English-blue" alt="English" /></a>
 <a href="./README.zh-CN.md"><img src="https://img.shields.io/badge/lang-简体中文-blue" alt="简体中文" /></a>
 <a href="./README.ja-JP.md"><img src="https://img.shields.io/badge/lang-日本語-blue" alt="日本語" /></a>
 <a href="./README.ko.md"><img src="https://img.shields.io/badge/lang-한국어-blue" alt="한국어" /></a>
 <a href="./README.de.md"><img src="https://img.shields.io/badge/lang-Deutsch-blue" alt="Deutsch" /></a>
</p>

> **Frühe Beta**: Wird aktiv weiterentwickelt. Mit Ecken und Kanten ist zu rechnen.

> **Lokal + verwaltete Dienste, upfront:** OpenHuman speichert seinen Memory Tree, Obsidian-Style-Markdown-Vault, Workspace-Konfiguration und lokalen Laufzeitstatus auf deiner Maschine. Die standardmäßige verwaltete Erfahrung nutzt weiterhin OpenHuman-gehostete Dienste für Account-Anmeldung, Model-Routing, Web-Search-Proxying und verwaltete Integration/OAuth-Flows über die Composio-Connector-Schicht. Wähle benutzerdefinierte/lokale Einstellungen, wenn du dein eigenes Modell, deine eigene Suche oder Composio-Credentials mitbringen möchtest; einige Echtzeit-Trigger und gehostete Funktionen erfordern weiterhin das verwaltete Backend.

Für Installation und Einstieg lade die App von [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) herunter oder führe im Terminal aus:

```bash
# DMG/EXE über https://tinyhumans.ai/openhuman herunterladen oder direkt im Terminal:

# macOS oder Linux x64
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash

# Windows
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

<!-- TODO: translate (de) — English source mirrored from README.md so non-EN readers get the same install caveats. Please translate. -->
> **Linux:** the AppImage can crash on launch under Wayland (and on Arch-based distros with `sharun: Interpreter not found!`) — see [#2463](https://github.com/tinyhumansai/openhuman/issues/2463) for the cause and env-var workarounds.
Arch Linux package maintainers can use the [`openhuman-bin` AUR recipe](./packages/arch/openhuman-bin/);
once published, Arch users can install it with `yay -S openhuman-bin`.
<!-- /TODO -->

# Was ist OpenHuman?

OpenHuman ist ein quelloffener, agentenbasierter Assistent, der sich in deinen Alltag einfügt. Jeder Punkt verlinkt auf die ausführliche Beschreibung in der [Doku](https://tinyhumans.gitbook.io/openhuman/).

- **Schlicht, UI-zuerst & menschlich** — Eine aufgeräumte Desktop-Erfahrung und kurze Onboarding-Pfade bringen dich in wenigen Klicks von der Installation zum laufenden Agenten — keine Config-First-Einrichtung, kein Terminal nötig. Der Agent hat [ein Gesicht](https://tinyhumans.gitbook.io/openhuman/features/mascot): ein Desktop-Maskottchen, das spricht, auf seine Umgebung reagiert, als echter Teilnehmer [in deinen Google-Meets sitzt](https://tinyhumans.gitbook.io/openhuman/features/mascot/meeting-agents), sich über Wochen an dich erinnert und im Hintergrund weiterdenkt, auch wenn du längst nicht mehr tippst.

- **[118+ Drittanbieter-Integrationen](https://tinyhumans.gitbook.io/openhuman/features/integrations) mit [Auto-Fetch](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki/auto-fetch)**: Gmail, Notion, GitHub, Slack, Stripe, Calendar, Drive, Linear, Jira und der Rest deines Stacks per **Ein-Klick-OAuth** anbinden. Jede Verbindung wird dem Agenten als typisiertes Tool freigegeben, und alle zwanzig Minuten geht der Core durch jede aktive Verbindung und zieht frische Daten in den [Memory Tree](https://tinyhumans.gitbook.io/openhuman/features/integrations/auto-fetch). Keine Prompts, keine Polling-Schleifen, die du selbst schreiben musst — der Agent hat morgens schon den Kontext für den Tag.

  Verwaltete Integrationen nutzen OpenHumans Composio-Connector-Schicht. OAuth-Handshakes und Integration-Tool-Calls werden standardmäßig über das verwaltete Backend geproxied. Wenn du stattdessen Composio direkt betreiben möchtest, konfiguriere den Direktmodus mit deinem eigenen Composio-API-Key; Echtzeit-Trigger-Webhooks müssen dann von dir selbst gehostet und verkabelt werden.

- **[Memory Tree](https://tinyhumans.gitbook.io/openhuman/features/memory-tree) + [Obsidian-Wiki](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki)**: eine lokal-zentrierte Wissensbasis, aufgebaut aus deinen Daten und deinen Aktivitäten. Alles, was du verbindest, wird in Markdown-Chunks von ≤3k Tokens kanonisiert, bewertet und in hierarchische Zusammenfassungs-Bäume gefaltet, gespeichert in **SQLite auf deiner Maschine**. Dieselben Chunks landen als `.md`-Dateien in einem Obsidian-kompatiblen Vault, das du öffnen, durchstöbern und editieren kannst — inspiriert von Karpathys [obsidian-wiki-Workflow](https://x.com/karpathy/status/2039805659525644595).

- **Alles eingebaut**: Web-Suche, ein Web-Fetch-[Scraper](https://tinyhumans.gitbook.io/openhuman/features/native-tools), ein vollständiges Coder-Toolset (Dateisystem, Git, Lint, Test, Grep) und [native Sprache](https://tinyhumans.gitbook.io/openhuman/features/voice) (STT als Eingabe, ElevenLabs TTS als Ausgabe, Lippensynchronisation für das Maskottchen, Live-Google-Meet-Agent) sind ab Werk verdrahtet. Standardmäßig nutzt [Model-Routing](https://tinyhumans.gitbook.io/openhuman/features/model-routing) das OpenHuman-Backend, um das passende LLM für jede Workload auszuwählen und zu proxien (Reasoning, Fast oder Vision). Ein Abo umfasst alle Modelle. Keine "erst-ein-Plugin-installieren-um-Dateien-zu-lesen"-Hürde. [Optional lokale KI über Ollama](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-ai) für On-Device-Workloads.

- **[Smarte Token-Kompression (TokenJuice)](https://tinyhumans.gitbook.io/openhuman/features/token-compression)**: Jeder Tool-Aufruf, jedes Scrape-Ergebnis, jeder E-Mail-Text und jeder Such-Payload läuft durch eine Token-Kompressionsschicht, bevor er ein LLM-Modell erreicht. HTML wird zu Markdown konvertiert, lange URLs werden gekürzt, und ausschweifende Tool-Ausgaben werden über eine konfigurierbare Regel-Ebene dedupliziert und zusammengefasst usw. CJK, Emojis und andere Multi-Byte-Texte bleiben Graphem für Graphem erhalten — niemals abgeschnitten. Du erhältst dieselbe Information bei einem Bruchteil der Tokens. Kosten und Latenz sinken um bis zu 80%.

- **[Messaging-Kanäle](https://tinyhumans.gitbook.io/openhuman/features/integrations#messaging-channels)** und **[Privatsphäre & Sicherheit](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security)**: ein- und ausgehend über die Kanäle, die du ohnehin nutzt — Workflow-Daten bleiben auf dem Gerät, lokal verschlüsselt, und gehören dir.

## Beitragen aus dem Quellcode

Neu hier? Beginne mit [`CONTRIBUTING.md`](./CONTRIBUTING.md) für den Fork-/PR-Workflow und die lokalen Prüfbefehle. Der kurze Weg:

1. Installiere Git, Node.js 24+, pnpm 10.10.0, Rust 1.93.0 (`rustfmt` + `clippy`), CMake, Ninja, ripgrep sowie die plattformspezifischen Desktop-Build-Voraussetzungen.
2. Forke und klone das Repo, führe dann `git submodule update --init --recursive` aus, bevor du `pnpm install` startest, damit die mitgelieferten Tauri/CEF-Quellen vorhanden sind.
3. Nutze `pnpm dev` für reine Web-UI-Arbeit, `pnpm --filter openhuman-app dev:app` für die Desktop-Shell sowie gezielte Checks wie `pnpm typecheck`, `pnpm format:check` und `cargo check -p openhuman --lib`, bevor du einen PR öffnest.

Tiefer einsteigen: [Architektur](https://tinyhumans.gitbook.io/openhuman/developing/architecture) · [Einrichtung](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up) · [Cloud-Deployment](./gitbooks/features/cloud-deploy.md).

## Kontext in Minuten, nicht in Wochen

OpenHuman ist das erste Agent-Harness, das dich in Minuten kennenlernt. Inspiriert von [Karpathys LLM-Knowledgebase](https://x.com/karpathy/status/2039805659525644595). Die meisten Agenten starten aus dem Kalten. Hermes lernt, indem er dir bei der Arbeit zusieht; OpenClaw wartet darauf, dass Plugins Kontext einspielen. So oder so vergehen Tage oder Wochen, bevor der Agent genug über deinen Stack weiß, um wirklich nützlich zu sein.

<p align="center">
 <img src="./gitbooks/.gitbook/assets/image (1).png" alt="Diagramm zum OpenHuman-Kontextaufbau" />
</p>

> OpenHuman fasst all deine Dokumente, E-Mails und Chats zusammen, komprimiert sie und legt einen Memory Graph an, mit dem dein Agent sich alles über dich merken kann.

OpenHuman überspringt die Wartezeit. Verbinde deine Accounts, lass [Auto-Fetch](https://tinyhumans.gitbook.io/openhuman/features/integrations/auto-fetch) die Daten lokal in einer 20-Minuten-Schleife abholen, und [Memory Trees](https://tinyhumans.gitbook.io/openhuman/features/memory-tree) komprimieren alles in Markdown-Dateien, intelligent abgelegt in einem [Obsidian-Wiki im Karpathy-Stil](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki).

Nach nur einem Sync-Durchlauf hat der Agent den vollständigen (komprimierten) Kontext deines Postfachs, deines Kalenders, deiner Repos, deiner Dokumente und deiner Nachrichten. Keine Trainingsphase. Kein "gib ihm ein paar Wochen". Er wird zu dir — gesteuert von dir.

Du hostest [agentmemory](https://github.com/rohitg00/agentmemory) bereits selbst für andere Coding-Agenten? OpenHuman bringt ein optionales `Memory`-Backend mit, das dorthin proxyt — setze `memory.backend = "agentmemory"` in `config.toml`, und derselbe persistente Store treibt OpenHuman zusammen mit Claude Code, Cursor, Codex und OpenCode an. Setup-Details auf der Seite zum [agentmemory-Backend](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki/agentmemory-backend).

## OpenHuman vs. andere Agent-Harnesses

Übersichtsvergleich (Produkte entwickeln sich weiter — bitte beim jeweiligen Anbieter verifizieren). OpenHuman ist darauf ausgelegt, **Vendor-Wildwuchs zu reduzieren**, **Workflow-Wissen auf dem Gerät zu halten** und dem Agenten eine **persistente Erinnerung** an deine Daten zu geben — nicht nur an den Chat.

|                     | Claude Cowork     | OpenClaw          | Hermes Agent      | OpenHuman                          |
| ------------------- | ----------------- | ----------------- | ----------------- | ---------------------------------- |
| **Quelloffen**      | 🚫 Proprietär     | ✅ MIT            | ✅ MIT            | ✅ GNU                             |
| **Einfacher Einstieg** | ✅ Desktop + CLI | ⚠️ Terminal zuerst | ⚠️ Terminal zuerst | ✅ Aufgeräumte UI, in Minuten   |
| **Kosten**          | ⚠️ Abo + Zusatzkosten | ⚠️ BYO-Modelle | ⚠️ BYO-Modelle | ✅ Ein Abo + TokenJuice            |
| **Memory**          | ✅ chat-gebunden  | ⚠️ plugin-abhängig | ✅ selbstlernend | 🚀 Memory Tree + Obsidian-Vault, optional [agentmemory](https://github.com/rohitg00/agentmemory)-Backend |
| **Integrationen**   | ⚠️ wenige Konnektoren | ⚠️ BYO          | ⚠️ BYO            | 🚀 118+ über OAuth                 |
| **Auto-Fetch**      | 🚫 keiner         | 🚫 keiner         | 🚫 keiner         | ✅ 20-Min.-Sync ins Memory         |
| **API-Wildwuchs**   | 🚫 zusätzliche Keys | 🚫 BYOK         | 🚫 Multi-Vendor   | ✅ ein Account                     |
| **Model-Routing**   | 🚫 nur ein Modell | ⚠️ manuell        | ⚠️ manuell        | ✅ eingebaut                       |
| **Native Tools**    | ✅ nur Code       | ✅ nur Code       | ✅ nur Code       | ✅ Code + Suche + Scraper + Sprache |

# Gib uns einen Stern auf GitHub

_Baust du auch in Richtung AGI und künstlichem Bewusstsein? Setze einen Stern und hilf anderen, den Weg zu finden._

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

# Contributors Hall of Fame

Zeig etwas Liebe und lande in der Hall of Fame. Mitwirkende erhalten kostenloses Merch und besonderen Zugang zu unserem [Discord](https://discord.tinyhumans.ai/).

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
