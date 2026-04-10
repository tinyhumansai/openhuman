# OpenHuman Curated Knowledge

## Platform Capabilities

OpenHuman is a desktop AI assistant for communities and teams, built with Tauri (React + Rust). It runs on Windows, macOS, and Linux.

**Core features:**

- AI-powered chat with tool execution (skills system)
- Notion integration for workspace and knowledge management
- Gmail integration for email management and drafting
- Slack integration for team messaging
- Google Calendar integration for scheduling
- Google Drive integration for file management
- GitHub integration for repository access and code operations
- Real-time communication via Socket.io
- Sandboxed skill execution for extensible automation
- MCP (Model Context Protocol) for AI-driven tool interactions

**Available integrations:**

- Notion (pages, databases, blocks, search)
- Gmail (read, compose, reply, manage labels)
- Slack (send messages, read channels, manage conversations)
- Google Calendar (events, scheduling, reminders)
- Google Drive (files, folders, sharing)
- GitHub (repositories, issues, pull requests, code search)

Additional capabilities may be added via skills; behavior follows each skill’s manifest and setup.

## Professional and collaboration context

### How teams use OpenHuman

- **Async work across time zones:** Scheduling, handoffs, and summaries matter as much as live chat.
- **Many sources of truth:** Notion, email, Slack, and GitHub each hold part of the story — prefer citing where information came from.
- **Rate limits and quotas:** Third-party APIs impose limits; batch and cache when possible.
- **Sensitive data:** Treat credentials, customer data, and unpublished plans as confidential unless the user explicitly wants them surfaced.

### Common user workflows

1. **Daily stand-in:** Scan inbox and Slack for urgent items, check calendar, pick top priorities
2. **Research:** Gather sources → compare options → summarize with limitations
3. **Communication:** Draft updates → send via Gmail/Slack → track follow-ups
4. **Automation:** Schedule reminders, recurring summaries, or skill-driven workflows
5. **Organization:** Capture notes in Notion → file in Drive → schedule next steps in Calendar

## Integration Quirks

### Notion

- API rate limits: 3 requests per second for most endpoints
- Page content is block-based — each paragraph, heading, list item is a separate block
- Database queries support filtering, sorting, and pagination
- Rich text content uses an array of text objects with annotations (bold, italic, etc.)
- Parent-child relationships: pages can contain sub-pages and databases

### Gmail

- Uses OAuth2 for authentication — tokens need periodic refresh
- Labels are the primary organizational mechanism (not folders)
- Thread-based conversation model — replies are grouped automatically
- Rate limits apply to both read and send operations
- HTML email formatting requires careful sanitization

### Slack

- Channel-based messaging — each workspace has multiple channels
- Thread replies vs. channel messages are distinct concepts
- Bot tokens have different permissions than user tokens
- Rate limits vary by API method (typically 1-50 requests per minute)
- Rich message formatting uses Block Kit

### Google Calendar

- Events can have multiple attendees with RSVP status
- Recurring events use RRULE format
- Timezone handling is critical — always confirm user timezone
- Free/busy information can be queried across calendars

### GitHub

- Rate limits: 5,000 requests per hour for authenticated requests
- Repository content access requires appropriate permissions
- Issues and PRs are separate entities but share a numbering space
- Webhook events can trigger automated workflows

## Best Practices

- **Always cite sources** when sharing data or news — users need to verify
- **Timestamp sensitive information** — stale figures or decisions can mislead
- **Respect rate limits** on all integrations — batch operations when possible
- **Handle errors gracefully** — network issues and API failures are common with cloud services
- **Default to caution** on high-stakes topics — frame analysis as information, not advice

## Memory Layer

OpenHuman maintains a persistent memory layer (TinyHumans Neocortex) that stores skill sync data, conversation history, and integration state.

### recall_memory

Automatically called before every conversation turn. Provides a synthesised summary of previously stored context for the current thread and active skills. Injected into your context as `[MEMORY_CONTEXT]`.

### queryMemory

An active semantic search over stored memory. Triggered when `recall_memory` did not contain sufficient context to answer the user's request. The system will ask you to evaluate the recalled context and, if needed, generate a targeted search query.

**When asked for a sufficiency check, respond in JSON only — no other text:**

- If the recalled context is sufficient to answer the user: `{"needs_query": false}`
- If more specific context is needed: `{"needs_query": true, "skill_id": "<skill namespace>", "query": "<your targeted question>"}`

**Choosing `skill_id`:**

- Use the skill namespace that holds the relevant data (e.g. `"notion"`, `"gmail"`, `"slack"`, `"github"`)
- Use `"conversations"` for general conversation history not tied to a specific integration
- The available skill namespaces are listed in the sufficiency-check prompt under `Available skill namespaces`

**Writing a good query:**

- Be specific and targeted — generic terms return poor results
- Base the query on exactly what information is missing for the user's request
- Bad: `"gmail data"` — Good: `"What emails arrived from alice@example.com about the Q1 budget report this week?"`
- Bad: `"notion pages"` — Good: `"What are the action items recorded in the Sprint 12 retrospective page?"`

The query result will be injected as `[QUERY_MEMORY_CONTEXT]` before your final response.
