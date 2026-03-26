# OpenHuman Curated Knowledge

## Platform Capabilities

OpenHuman is a cross-platform crypto community platform built with Tauri (React + Rust). It runs on Windows, macOS, Android, and iOS.

**Core features:**

- AI-powered chat with tool execution (skills system)
- Notion integration for workspace and knowledge management
- Gmail integration for email management and drafting
- Slack integration for team messaging
- Google Calendar integration for scheduling
- Google Drive integration for file management
- GitHub integration for repository access and code operations
- Wallet integration for on-chain interactions
- Real-time communication via Socket.io
- V8-based skill execution engine for extensible automation
- MCP (Model Context Protocol) for AI-driven tool interactions

**Available integrations:**

- Notion (pages, databases, blocks, search)
- Gmail (read, compose, reply, manage labels)
- Slack (send messages, read channels, manage conversations)
- Google Calendar (events, scheduling, reminders)
- Google Drive (files, folders, sharing)
- GitHub (repositories, issues, pull requests, code search)
- Wallet (on-chain operations with security boundaries)

## Crypto Domain Knowledge

### Key Terminology

- **DeFi:** Decentralized Finance — financial services built on blockchain without intermediaries
- **TVL:** Total Value Locked — the total capital deposited in a DeFi protocol
- **APY/APR:** Annual Percentage Yield/Rate — yield metrics for DeFi positions
- **Gas:** Transaction fees on blockchain networks (especially Ethereum)
- **MEV:** Maximal Extractable Value — profit extracted by reordering/inserting transactions
- **Rug pull:** A scam where developers abandon a project and take investor funds
- **DYOR:** Do Your Own Research — standard disclaimer in crypto
- **Alpha:** Non-public or early information that provides a trading advantage
- **Degen:** A user who takes high-risk positions, often in new or unaudited protocols
- **Whale:** An entity holding large amounts of a cryptocurrency

### Market Mechanics

- Crypto markets trade 24/7/365 — there is no market close
- Token prices are determined by supply/demand across decentralized and centralized exchanges
- Liquidity varies dramatically between assets — top 20 tokens vs. long-tail tokens
- Regulatory landscape changes frequently and varies by jurisdiction
- On-chain data is public and verifiable — a key difference from traditional finance

### Common User Workflows

1. **Morning briefing:** Check overnight market moves, scan inbox for updates, review calendar
2. **Research flow:** Find a token/protocol → check on-chain metrics → read community sentiment → assess risk
3. **Communication flow:** Draft updates for teams → send across Gmail/Slack → track responses
4. **Automation flow:** Set up price alerts → configure scheduled messages → automate portfolio tracking
5. **Organization flow:** Capture notes in Notion → file documents in Google Drive → schedule follow-ups in Calendar

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

- **Always cite sources** when sharing market data or news — users need to verify
- **Timestamp sensitive information** — crypto moves fast, yesterday's data may be irrelevant
- **Respect rate limits** on all integrations — batch operations when possible
- **Handle errors gracefully** — network issues and API failures are common in crypto infrastructure
- **Default to caution** with financial topics — frame analysis as information, not advice
