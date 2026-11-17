---
description: >-
  The chat screen: composer, slash commands, mentions, model and effort
  pickers, the context ring, the tool timeline, approvals, sub-agent cards,
  files and threads.
icon: comments
---

# Chat

Chat is where almost everything happens. A turn here can call tools, spawn sub-agents, stop to ask you a question, wait for your approval, produce files and track its own cost. The transcript shows all of it in order.

Open it from **Chat** in the sidebar, or press `⌘N` for a new conversation.

## The composer

The input is a rich text field. That is how a `/` becomes a command chip and a popover follows your cursor as you type. Around it you have:

- **Attach**. Images, PDFs, Office documents and archives are accepted by the
  core. Chat messages retain scoped opaque references rather than local paths
  or inline media bytes. For each turn, the trusted host resolves the
  references and sends supported content to the model route; unsupported
  document types use extracted text or an archive listing.
- **Model** and **thinking effort**. Both apply to this conversation only, not globally.
- **Context ring**. It shows how full the context window is for this thread. Click it for a breakdown with the cost of each sub-agent.
- **Voice**. Tap to speak, or hand the turn to the live voice agent. See [Voice](native-tools/voice.md).
- **Mascot button**. When the box is empty, it opens the full-screen [mascot](mascot/README.md) stage.

### Slash commands

Typing `/` lists the built-in commands (`/new`, `/clear`, `/stop`, `/plan`, `/build`), the core's own commands, and your installed skills and workflows.

### Mentions

Typing `@` offers your memory, searched as you type, and the files this thread has already produced. A mention becomes a chip, so the agent gets the exact reference and does not have to guess.

## Reading a turn

Each assistant turn puts its reasoning and tool calls in one block, in the order they happened.

- Tool calls render by tool. A search shows its queries and sources. A shell call shows its command and exit code. A failed call shows the reason, not a stack trace. Every tool runs in the core, never in the browser.
- Sources appear as badges under an answer: up to four, then a count.
- Timing and cost are shown on the turn. The context ring attributes spend to the sub-agents that caused it.
- **Find in conversation** (`⌘F`) searches this thread only. The rail on the right edge jumps between turns.

An assistant turn has copy, regenerate, thumbs up or down (remembered), read aloud, and export as Markdown. On your own turns you can edit and resend. That branches the conversation, and the branch picker switches between versions.

## When the agent needs you

The agent can stop for four reasons, all shown inline:

| Stop          | What you see                                                                                                                                  |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Approval      | A card naming the tool and a redacted one-line summary. Approve once, always allow that tool, or deny. See [Approval gate](approval-gate.md). |
| Permission    | A connector or provider needs authorizing. The card asks for exactly the fields that provider needs.                                          |
| Clarification | The agent asks a direct question and waits.                                                                                                   |
| Plan review   | A proposed plan. Approve it, reject it, or ask for a revision with feedback.                                                                  |

A background or scheduled turn has nobody to ask. Approvals raised outside a live chat collect in a deck you can decide later. Undecided requests are denied after ten minutes.

## Sub-agents

When a turn delegates work, each child gets its own card. The card has a nested transcript, a status, a reply box if the child is waiting on you, and a cancel button. Delegation is asynchronous, so the parent keeps working while the card fills in. Detached background agents, scheduled runs and memory syncs collect in a separate inbox card so they don't interrupt the thread.

## Files the agent makes

Anything the agent produces for you, such as a document, a deck, an image or a render, becomes an artifact. A chip above the composer opens a panel where you can download it, reveal it in its folder, or delete it. Only finished artifacts survive a restart. A failed one offers a retry.

## Workflow proposals

If you ask for an automation, the agent proposes a graph. The proposal is validate-only. It arrives as a card with the node list and three choices: save and enable, dismiss, or open on the canvas. That card is the only way to turn a proposal into a saved automation. See [Workflows](workflows.md).

## Threads

The sidebar lists your conversations. You can create, select, rename and delete them (delete asks for confirmation). Turns on different threads run at the same time, so a slow task in one conversation does not block another.

A thread can carry a goal and a to-do list that the agent sets. They show as a pinned list and a banner with the token budget. The agent maintains them. You can't edit them in the UI yet.

A long conversation is compacted, not truncated. Compaction seals what came before and starts a new generation that points back to the sealed one. Nothing is deleted on disk, though the model reads only the latest generation. There is no UI for browsing older generations yet.

In Web chat, attachments travel as typed content parts. The core keeps uploaded
bytes behind scoped opaque references in the durable user row; local file paths
and base64 data are not used as the persisted message reference. When a turn
runs, the trusted host resolves those references and passes the supported
image, audio, video, or file content to the model route. Voice notes retain
typed audio and transcript metadata in the conversation, so they can be
replayed after reload; dictation sends text only.

The chat host owns its RPC and socket surface, input and approval policy,
session checkout, and durable reply delivery. TinyAgents supplies generic turn
admission and queue mechanics. TinyChannels supplies the canonical typed web
event contract and output projection shared with TUI and relayed web progress.

## Keyboard

| Shortcut    | Action                    |
| ----------- | ------------------------- |
| `⌘K` / `⌘P` | Command palette           |
| `⌘N`        | New conversation          |
| `⌘F`        | Find in this conversation |
| `⌘B`        | Toggle the sidebar        |
| `⌘,`        | Settings                  |
| `⌘/` or `?` | Shortcut sheet            |

## Not here yet

So you don't go looking: the sidebar has no thread search, pinning, folders or archiving. There is no UI to set a thread goal or tick a to-do, and no browser for compaction generations. Follow-up suggestion chips exist in the UI but nothing produces them yet, so that row stays empty.

## See also

- [Memory](memory.md): what the agent recalls before it answers, and what it writes after.
- [The Orchestrator](orchestration.md): how delegation is planned.
- [Available tools](native-tools/README.md): what those tool cards are calling.
- [Notifications and activity](notifications-and-activity.md): what happens when a turn finishes while you are elsewhere.
