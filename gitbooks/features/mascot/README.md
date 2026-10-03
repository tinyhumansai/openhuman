---
description: >-
  The on-screen face of OpenHuman, a desktop mascot that speaks, reacts, joins
  your meetings, and thinks in the background even when you aren't looking at
  it.
icon: face-smile
---

# The Mascot

OpenHuman has a face. The mascot is an animated character that lives on your desktop and acts as the visible surface of the agent, what it's saying, what it's thinking about, when it's idle, when it's busy, when it has something to tell you.

It is not a chrome ornament. The mascot is wired into the same pieces as the rest of the agent: voice, memory, [scheduled runs](../native-tools/cron.md), and the [Google Meet integration](../native-tools/voice.md). When the agent talks, the mascot is the one talking; when the agent is thinking, the mascot is the one thinking.

## What it does

### It speaks, and lip-syncs to its own voice

When the agent replies, the audio is generated through a hosted TTS model and streamed to your speakers. At the same time, the mascot drives a viseme map against the audio so its mouth shapes match the words coming out. There's no separate "talking head" video, the same audio stream that you hear is the one driving the animation.

See [Native Voice](../native-tools/voice.md) for the speech-to-text, text-to-speech, and meeting plumbing the mascot rides on top of.

### It joins your meetings, as a real participant

### It moves and reacts to its surroundings

The mascot has mood states (idle, thinking, listening, talking, surprised) and it transitions between them based on what the agent is doing. When you start typing it shifts into a listening pose. When the model is reasoning, it shows that. When a tool call returns something noteworthy, it reacts. When you stop interacting for a while, it drifts into idle.

After a turn finishes, the desktop mascot also reads the conversation-level cue that arrives with the chat result. A success cue produces a short happy acknowledgement, uncertainty produces a confused acknowledgement, and warnings or failed outcomes produce a concerned acknowledgement. If no strong cue is present, it keeps the existing calm post-turn acknowledgement and falls back to idle.

It is meant to feel alive, not animated-on-rails.

### It remembers you

The mascot is the visible part of an agent that has [memory](../memory.md) underneath it. It remembers what you've talked about, what you prefer, what's in your documents and what's been decided, across the sources you've added. When it greets you in the morning, it isn't starting from zero.

That memory is what makes the personality consistent over weeks and months. The mascot you talk to today knows what the mascot you talked to last Tuesday knows.

### It works while you're away

Even when you've stopped typing, work can keep happening on your behalf: [scheduled routines](../native-tools/cron.md) run on a cron expression, [triggers](../integrations/triggers.md) fire on inbound events, and [workflows](../workflows.md) execute durable, approval-gated graphs. So when you come back to the desk, the mascot may have already drafted the email, refreshed the dashboard, or queued the question it needs to ask you. The face on the screen is the one that did the work.

## Why have a mascot at all?

Most assistants are a blinking text input. That's fine for a tool. It's not fine for something that's meant to be alongside you all day, with persistent memory of your life, taking actions on your behalf.

The mascot exists because:

- **Presence beats panels.** A face you can glance at tells you, in one frame, whether the agent is busy, idle, or trying to get your attention.
- **It makes voice calls feel like a conversation.** A camera feed of an animated character lip-syncing to its own speech is a different experience than a robotic voice with a black tile.
- **Personality is a UX surface.** A consistent character on screen is easier to trust, talk to, and forgive when it makes a mistake than a faceless API.

## See also

- [Native Voice](../native-tools/voice.md), the STT / TTS plumbing the mascot rides on.
- [Memory](../memory.md), what the mascot remembers, and how.
- [Chromium Embedded Framework](../../developing/cef.md), the camera-into-Meet pipeline (developer reference).
