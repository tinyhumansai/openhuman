---
icon: screwdriver
---

# Skills & Integrations

OpenHuman extends its capabilities through Skills, organized into five categories accessible from the Skills tab in the bottom navigation. Each Skill is either a built-in desktop capability, a messaging channel, or a third-party integration that connects through OAuth.

<figure><img src="../.gitbook/assets/9. Skills &#x26; Integrations@2x.png" alt=""><figcaption></figcaption></figure>

#### Built-in Skills

Core desktop capabilities that run locally on your device.

**Screen Intelligence:** Captures windows, summarizes what is on screen, and feeds useful context into memory. Configurable capture frequency (\~5 seconds default), per-app allowlist and denylist controls. All processing happens locally.

**Text Auto-Complete:** Suggests inline completions while you type and lets you control where autocomplete is active. Powered by your local AI model and your memory context. Press Tab to accept suggestions.

**Voice Intelligence:** Uses the microphone for dictation and voice-driven chat with your AI. All speech recognition runs locally on your device.

Each built-in Skill has a Setup or Enable button to activate it.

#### Channels

Messaging platforms that OpenHuman can send and receive messages through.

**Telegram:** Send and receive messages via Telegram. Full two-way access including reading history, sending and replying, forwarding, editing, deleting, managing groups, handling reactions, polls, contacts, admin actions, and more. Over 80 distinct actions are available.

**Discord:** Send and receive messages via Discord. Two-way messaging across your Discord servers and channels.

Both channels show "Not configured" until you click Setup and complete the connection flow.

#### Social

Connect social platforms to extend your knowledge graph and post or interact across channels.

**Facebook:** Create posts, manage pages, and work with Facebook social data.

**Instagram:** Manage Instagram publishing, messaging, and social content workflows.

**Reddit:** Read posts, monitor communities, and participate in Reddit discussions.

**Slack:** Send messages and read channel history in Slack.

#### Tools & Automation

Connect development and project management tools.

**GitHub:** Inspect repos, issues, and pull requests on GitHub.

**Linear:** Triage and update issues in your Linear workspace.

#### Productivity

Connect your work tools so OpenHuman can pull context from email, calendar, documents, and spreadsheets.

**Gmail:** Read, search, and send email through your Google account.

**Google Calendar:** List and manage events across your Google calendars.

**Google Drive:** Browse and fetch files from your Google Drive.

**Google Sheets:** Read, update, and organize spreadsheets in Google Sheets.

**Notion:** Read and edit pages, databases, and comments in Notion.

#### How Connections Work

Most third-party integrations connect via Composio OAuth. When you click Connect on an integration, a browser window opens for authentication. Once you complete sign-in, the connection becomes active and OpenHuman can begin syncing data.

Each integration shows its current status:

* **Not connected / Not configured:** Integration has not been set up.
* **Connected:** Integration is active and syncing.
* **Manage:** Active integration with options to reconfigure or disconnect.

#### What Skills Can Do

Skills operate within defined boundaries. A skill can fetch external data, run on a schedule, process and transform information, write structured outputs, and respond to events. Each skill runs in its own sandboxed environment with enforced resource limits.

#### Privacy

When you connect a Skill, OpenHuman has the access scope shown for that integration. All ingested data is processed and compressed locally on your device. Raw data does not leave your machine. Compressed summaries and structured context enter your Neocortex knowledge graph with AES-256-GCM encryption.

You can revoke any connection at any time from the Skills tab.
