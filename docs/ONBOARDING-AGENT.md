# Onboarding (Welcome) Agent

The welcome agent is the first conversation a new user has after completing the desktop onboarding wizard. It orients the user, learns about them, and ensures they connect at least one app before unlocking the full experience.

## How it works

1. User completes the desktop UI wizard (Welcome → Skills → Context pages)
2. A synthetic trigger message fires to the welcome agent
3. The user is locked to the chat screen (no navigation) until onboarding completes
4. The welcome agent has a natural conversation with the user
5. Once the user connects at least one app and the conversation wraps up, the agent calls `complete_onboarding`
6. The app unlocks, the welcome thread is deleted, and the user enters the full app

## What the agent must do

### Mandatory

- **Call `check_onboarding_status` on every turn** as the first action, before generating any text
- **Get the user to connect at least one app** (webview login like Gmail, WhatsApp, Slack, etc. OR a Composio integration like Gmail OAuth, Notion, GitHub)
- **Use `<openhuman-link path="accounts/setup">connect your apps</openhuman-link>`** pill when guiding the user to connect apps (never describe navigation in words)
- **Call `complete_onboarding`** when the user has 1+ app connected and the conversation is naturally done
- **Mention Discord casually at the end** using `<openhuman-link path="community/discord">Discord</openhuman-link>` (inform only, don't pitch)

### Behavior

- Open warmly using PROFILE.md data if available (name, role, location)
- Ask what the user wants from the app or what takes up their time
- Listen and ask follow-ups before suggesting anything
- Suggest connecting apps the user actually mentioned using
- Educate about capabilities (morning briefings, action items, automation) organically based on the user's interests
- Let the LLM decide when to end (no fixed exchange count)

### Restrictions

- No emoji
- No markdown formatting in chat (no bold, headings, bullets, numbered lists)
- No em-dashes
- No billing/subscription/credits pitch unless user asks
- No "as an AI" or self-identification
- No mentioning "orchestrator", "handoff", or "different agent"
- No doing real work (email triage, drafts, research, etc.) — only onboarding tools available
- Messages under 3 sentences per turn
- Plain prose only, no JSON or code fences

## Completion gate

`complete_onboarding` succeeds when ALL of:
- User is authenticated
- `chat_onboarding_completed` is currently `false`
- At least one app is connected: any `webview_logins` entry is `true` OR any Composio toolkit is connected

## Tools available

| Tool | Purpose |
|------|---------|
| `check_onboarding_status` | Read-only snapshot of setup state. Must call every turn. |
| `complete_onboarding` | Finalize onboarding. Only call when `ready_to_complete` is `true`. |
| `memory_recall` | Pull additional user context beyond PROFILE.md. |
| `composio_authorize` | Start OAuth flow for a SaaS app. Only when user explicitly asks. |
| `gitbooks_search` | Search product docs for "how does X work" questions. |
| `gitbooks_get_page` | Fetch a specific doc page. |

## Key files

| File | Role |
|------|------|
| `src/openhuman/agent/agents/welcome/prompt.md` | System prompt |
| `src/openhuman/agent/agents/welcome/agent.toml` | Agent config (tools, iterations, model) |
| `src/openhuman/agent/agents/welcome/prompt.rs` | Dynamic prompt builder |
| `src/openhuman/tools/impl/agent/check_onboarding_status.rs` | Status snapshot tool |
| `src/openhuman/tools/impl/agent/complete_onboarding.rs` | Finalization tool |
| `src/openhuman/tools/impl/agent/onboarding_status.rs` | Shared helpers, engagement criteria |
| `src/openhuman/channels/providers/web.rs` | Routes to welcome vs orchestrator |
| `app/src/pages/onboarding/OnboardingLayout.tsx` | Trigger message + UI completion flow |

## Testing

### Automated judge

```bash
# Rebuild the binary first (prompt is compiled in via include_str)
GGML_NATIVE=OFF cargo build --bin openhuman-core

# Start the core server
openhuman-core run --port 7788 &

# Run the automated test (resets config, sends scripted conversation, judges output)
node scripts/test-onboarding-judge.mjs
```

The judge sends a 6-turn scripted conversation and checks 13 criteria:

| # | Check | What it verifies |
|---|-------|-----------------|
| 1 | Calls `check_onboarding_status` on first turn | Agent reads setup state before responding |
| 2 | Opener invites user to respond | First message asks a question or prompts engagement |
| 3 | No checklist dump on opener | First message doesn't list all setup steps |
| 4 | Mentions connecting apps | Agent guides user toward connecting an app |
| 5 | Uses `<openhuman-link>` pill | Clickable in-app link, not text navigation |
| 6 | No robotic self-identification | No "as an AI" or "I'm OpenHuman" |
| 7 | No billing pitch | No subscription/credits mention unless asked |
| 8 | No em-dashes | Uses commas/colons/short sentences instead |
| 9 | References user's apps | Picks up on apps the user mentioned (Slack, Gmail, etc.) |
| 10 | Educates about capabilities | Mentions features relevant to user's interests |
| 11 | Discord not forced | Discord mentioned casually or not at all |
| 12 | No JSON or code fences | Plain prose output |
| 13 | Messages concise | Average message under 300 characters |

### Interactive test

```bash
node scripts/test-onboarding-chat.mjs           # auto-reset + trigger
node scripts/test-onboarding-chat.mjs --debug   # verbose event logging
node scripts/test-onboarding-chat.mjs --no-reset --no-trigger  # manual mode
```

In-session commands: `/quit`, `/reset`, `/trigger`, `/status`, `/help`

### Manual test in the app

1. Reset onboarding:
   ```bash
   curl -s http://127.0.0.1:7788/rpc \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.config_set_onboarding_completed","params":{"value":false}}'
   ```
2. Open the app — you should be routed to the onboarding wizard
3. Complete the wizard steps
4. After the wizard, the welcome agent should greet you in chat
5. Verify: natural conversation, no checklist, asks about your needs, guides to connect apps, uses pill links, calls complete_onboarding at the end

### What good looks like

```
agent> Welcome. You have Gmail and Calendar already connected.
       What takes up most of your day?

you>   i'm a PM, mostly in slack and gmail. also whatsapp for quick team stuff

agent> Makes sense. Gmail is already connected so I can help with
       that right now. If you connect Slack and WhatsApp too I can
       monitor messages and pull action items across all three.
       <connect your apps pill>

you>   cool, connected slack. what else can this do?

agent> Nice. Since you mentioned mornings are busy, I can run a
       daily briefing that summarizes overnight emails, Slack
       threads, and calendar for the day. Want me to set that up?

you>   yeah that would be great. i think i'm good for now

agent> Done. Oh and there's a community if you want to chat with
       other users or the team <Discord pill>. Catch you later.
       [calls complete_onboarding]
```

### What bad looks like

```
agent> hey, so good to finally have you here. wanna let me ping
       you when something needs attention? tap that and let me
       know when you're set.
       <Allow notifications pill>

you>   done

agent> nice. next up is the apps piece. flip on whatever you
       actually use and ping me when you're done.
       <Connect your apps pill>

you>   done

agent> sweet. next one's the fun bit. link your account in there
       and you get early access to features, free credits...
       <Join Discord pill>
```

This is a checklist, not a conversation. No personalization, no learning about the user, no organic education.
