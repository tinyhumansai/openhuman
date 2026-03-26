---
icon: screwdriver
---

# Skills & Integrations

OpenHuman extends its capabilities through two systems: a **skills engine** that adds new functionality to the assistant, and **integrations** that connect OpenHuman to external tools. Together, they allow OpenHuman to do more than analyze conversations; it can fetch data, run scheduled tasks, produce structured outputs, and write results to the tools you already use.

---

## Skills Engine

Skills are sandboxed modules that run inside OpenHuman's native application. Each skill is an isolated unit with its own execution environment, storage, and permissions. Skills extend what the assistant can do without requiring an app update.

When you make a request, OpenHuman's AI automatically discovers which skills are available and decides whether to invoke them based on what you asked for. You do not need to manage skills manually. They are loaded, registered, and made available to the AI system transparently.

### What Skills Can Do

Skills operate within defined boundaries. A skill can:

- **Fetch external data:** make HTTP requests to APIs, retrieve information from services, and bring data into OpenHuman's context
- **Run on a schedule:** execute at defined intervals using cron-style scheduling, enabling automated monitoring, reporting, or data collection
- **Process and transform:** analyze, filter, or restructure data within its sandboxed environment, using its own local database for persistence
- **Write outputs:** produce structured results that the AI can present to you or export through integrations
- **Respond to events:** react to real-time events from the server, enabling workflows that trigger based on external conditions

### Isolation and Safety

Each skill runs in its own sandboxed environment with enforced resource limits. Skills cannot access each other's data or execution context. This means a misbehaving skill cannot affect the rest of the application or compromise your data.

Skills are discovered from a curated registry and filtered by platform compatibility. Only skills that support your current platform are loaded.

{% hint style="info" %}
Skills extend OpenHuman's capabilities without requiring trust beyond their declared scope. Each one is isolated, resource-limited, and independently revocable.
{% endhint %}

---

## Integrations

Integrations connect OpenHuman to external tools so structured outputs can live where your team already works. Integrations are not about importing data into OpenHuman, they are about exporting intelligence out of it.

### Supported Integrations

**Telegram:** OpenHuman connects to your Telegram account to read and analyze conversations, extract signals, summarize discussions, and generate context-aware responses. Access is scoped to the conversations you reference in your requests.

**Notion:** Export summaries, action items, decisions, and workflow records to your Notion workspace. OpenHuman writes structured documents that preserve the context of where outputs originated. Useful for building shared knowledge bases or living records.

**Google Sheets:** Export tabular data such as logs, reports, extracted metrics, or tracking records. Useful for lightweight analysis, reporting, and audit trails.

These integrations are optional and can be enabled or disabled at any time from your settings.

### How Data Moves

Data moves from OpenHuman to external tools **only when you explicitly request it**.

When a workflow, summary, or analysis is exported, OpenHuman writes structured data, not raw conversation text. There is no continuous sync, no background polling, and no automatic data transfer. Each export is a deliberate action you initiate.

Integrations are write-oriented by default. OpenHuman does not read from external tools unless a specific workflow requires it and you have approved that behavior.

{% hint style="info" %}
**You control what leaves OpenHuman.** Nothing is exported to external tools without a clear, user-initiated action.
{% endhint %}

---

## From Conversation to Action

One of OpenHuman's core capabilities is turning unstructured conversations into structured, actionable outputs.

When discussions lead to decisions, commitments, or tasks, OpenHuman can recognize these moments (when you ask it to analyze a conversation) and represent them as **workflows as** structured objects that capture what needs to happen, who is involved, and where the decision came from.

Workflows link back to the conversations they originated from, preserving intent and rationale. When connected to tools like Notion or Google Sheets, workflows can be exported without duplicating entire conversation histories.

Workflows exist to reduce coordination friction, not to replace dedicated project management systems. OpenHuman does not auto-assign tasks, enforce deadlines, or create workflows without your input.

---

## Intelligence Concepts

OpenHuman works by translating raw conversation activity into a small number of structured concepts. These concepts allow the system to reason about communication, coordination, and trust without storing message histories.

**Signals:** Structured interpretations extracted from conversations. A signal might represent a decision, warning, important update, or emerging theme. Signals are independent objects that can be filtered, referenced, or aggregated.

**Claims:** Specific assertions that can be evaluated over time, such as predictions, commitments, or verifiable statements. Claims are tracked structurally and can resolve as accurate, inaccurate, or inconclusive.

**Contributions:** Meaningful actions taken by participants within conversations, inferred from behavior and context. Contribution records support community recognition and value attribution.

**Workflows:** Structured processes that originate from conversation, preserving the context of decisions and commitments as actionable objects.

These concepts allow OpenHuman to support trust, coordination, and intelligence without becoming invasive or requiring the storage of raw conversation data.
