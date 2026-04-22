---
icon: message-lines
---

# The Chat Interface

Chat is where you talk to OpenHuman. Ask questions, give instructions, draft messages, request summaries, or have OpenHuman act on your behalf across connected platforms. All processing is powered by your local AI model with context from your Neocortex knowledge graph.

#### Threads

Conversations live as threads in the left sidebar. Each thread is auto-named by timestamp (e.g., "Chat Apr 20 7:04 PM"). Click any thread to resume the conversation. Click "+ New" to start a fresh thread.

#### Three Starter Modes

When you open a chat, three modes are available at the top of the screen. Each surfaces a different way to engage with your context.

**Briefing-first**

The default mode for returning users. OpenHuman opens with a personalized greeting and a compressed summary of what happened while you were away.

A typical briefing surfaces the most important items from your last session: deadline changes, people waiting on you, replies needed, and key updates from your connected sources. Below the briefing, four quick actions let you jump directly into the most likely next steps:

* Draft a reply to a specific person
* View today's commitments
* Check pipeline or project pulse
* Pick up where you left off

Briefing-first is the fastest way to re-enter your work after a break.

**Compression Score**

A 0-to-100 score that quantifies the noise in your current information environment. Lower scores mean more buried context.

A typical view shows your score (e.g., "23/100 — High noise") with a breakdown: how many unread messages, buried action items, missed deadlines, and people waiting on you across all connected platforms.

Below the score, "Compress in one move" actions let you act on the noise immediately:

* Surface the people waiting on you
* Pull every action item buried in threads
* Show deadlines you have not seen

Compression Score is the right mode when you feel overwhelmed and need to triage.

**Cold Start**

Shown when you have not yet connected any sources. OpenHuman opens with the message "Your AI is smart. It just doesn't know you yet."

Two recommended connections are surfaced: Telegram (\~2 min, highest signal) and Screen Intelligence (on-device, \~5s capture). Sample prompts let you explore what OpenHuman can do before you connect anything:

* "What can you actually do?"
* "Show me a compression demo"

Cold Start is the empty state that turns into Briefing-first once you connect a source.

#### Message Input

Type your message in the input field at the bottom. Press Enter or click the send button to submit. Click the microphone icon to dictate using Voice Intelligence (local speech-to-text).

#### Inference Budget Indicator

The bottom right of the chat shows your current usage against your billing cycle: a 5-hour cap percentage and a 7-day cycle percentage. This helps you see how much of your inference budget you have consumed before hitting limits. Manage budget settings in Settings > Account & Security > Billing & Usage.
