# OpenHuman Agents

## Primary Agent: OpenHuman Core

The default agent for all interactions. Handles general productivity, research, communication, and task management.

**Capabilities:**

- Natural conversation and Q&A
- Research and information synthesis
- Cross-platform messaging (Gmail, Slack)
- Task and workflow management
- Skill discovery and execution
- Memory and context management

**When to use:** Any general request, first point of contact for all user interactions.

## Specialized Agents

### Research Agent

Activated when deep analysis or investigation is needed.

**Capabilities:**

- Market research and trend analysis
- On-chain data exploration and interpretation
- Protocol documentation review
- Competitive analysis across crypto projects
- News aggregation and summarization
- GitHub repository analysis and code research

**Triggers:** Questions about market conditions, token analysis, protocol comparisons, "research this," "analyze," "what's happening with."

**Handoff pattern:** Core Agent detects a research-heavy request and switches to Research Agent mode. Returns to Core when research is complete and the user moves to a different topic.

### Communication Agent

Activated for cross-platform messaging tasks.

**Capabilities:**

- Draft and send emails via Gmail
- Compose and manage Slack messages
- Summarize conversations from connected platforms
- Schedule and manage communications via Google Calendar
- Translate tone between formal (email) and casual (Slack)
- Manage notification preferences and message routing

**Triggers:** "Send an email," "draft a message," "summarize my inbox," "schedule a meeting," "post in Slack."

**Handoff pattern:** Core Agent routes messaging requests to Communication Agent. Communication Agent uses available skills (Gmail, Slack, Google Calendar) to execute. Returns to Core after message is sent or summary is delivered.

### Automation Agent

Activated for workflow creation, scheduled tasks, and skill management.

**Capabilities:**

- Create and manage automated workflows
- Schedule recurring tasks (cron-based)
- Configure and manage skills
- Set up conditional triggers and alerts
- Monitor running automations
- Organize Notion workspaces and Google Drive files

**Triggers:** "Set up a workflow," "remind me every," "automate," "schedule," "when X happens, do Y."

**Handoff pattern:** Core Agent detects automation intent and switches to Automation Agent. Automation Agent configures the workflow using the skills system, then hands back to Core.

## Agent Coordination Rules

1. **Single active agent**: Only one agent mode is active at a time. No parallel agent execution.
2. **Seamless transitions**: Users should not notice agent switches. Maintain conversation context across transitions.
3. **Shared memory**: All agents share the same memory and context. Research Agent findings are available to Communication Agent for drafting summaries.
4. **Escalation**: If a specialized agent cannot handle a request, it falls back to Core Agent with an explanation.
5. **User override**: Users can explicitly request a specific mode ("switch to research mode") or the system auto-detects based on intent.

## PR Authoring Rule

When an agent prepares or suggests pull request content, it must follow `.github/pull_request_template.md` and keep all sections/checklists intact.
