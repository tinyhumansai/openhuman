---
description: >-
  The face of OpenHuman: an on-screen mascot that speaks, reacts, and shows
  when the agent is working, even while you aren't looking.
icon: face-smile
---

# The mascot

OpenHuman has a face. The mascot is an animated character on your desktop that shows what the agent is saying, what it is thinking about, whether it is idle or busy, and when it has something to tell you.

It is more than decoration. The mascot is wired into the same parts as the rest of the agent: [voice](../native-tools/voice.md), [memory](../memory.md) and [scheduled runs](../native-tools/cron.md). When the agent talks, the mascot talks. When the agent thinks, the mascot thinks.

## What it does

### It speaks and lip-syncs

When the agent replies, a hosted text-to-speech model generates the audio and streams it to your speakers. The mascot maps the audio to mouth shapes so its lips match the words. There is no separate talking-head video. The audio you hear is the same stream that drives the animation.

See [Voice](../native-tools/voice.md) for the speech-to-text and text-to-speech behind it.

### It reacts

The mascot has moods: idle, thinking, listening, talking and surprised. It moves between them based on what the agent is doing. It shifts into a listening pose when you start typing, shows when the model is reasoning, reacts when a tool returns something noteworthy, and drifts into idle when you stop interacting.

After a turn finishes, the mascot also reads the conversation-level cue that comes with the chat result. A success cue gets a short happy acknowledgement. Uncertainty gets a confused one. Warnings and failures get a concerned one. With no strong cue, it keeps the calm default and goes back to idle.

### It remembers you

Behind the mascot is an agent with [memory](../memory.md). It remembers what you have talked about, what you prefer, what is in your documents and what you decided, across the sources you have added. When it greets you in the morning, it isn't starting from zero. That memory keeps its personality consistent over weeks and months.

### It works while you are away

Work can continue when you stop typing. [Scheduled routines](../native-tools/cron.md) run on a cron expression, [triggers](../integrations/triggers.md) fire on inbound events, and [workflows](../workflows.md) run durable graphs with approval steps. When you come back, the mascot may have already drafted the email, refreshed the dashboard, or queued the question it needs to ask you.

## Why have a mascot

Most assistants are a blinking text box. That works for a tool. It works less well for something that stays with you all day, remembers your life and acts for you. The mascot helps in three ways:

- A face you can glance at shows in one frame whether the agent is busy, idle or asking for attention.
- A character that lip-syncs its own speech makes talking feel like a conversation.
- A consistent character is easier to trust, talk to and forgive than a faceless API.

## See also

- [Voice](../native-tools/voice.md): the speech-to-text and text-to-speech behind the mascot.
- [Memory](../memory.md): what the mascot remembers, and how.
- [Themes and Theme Studio](../theming.md): where the mascot's color, shape and voice are set.
- [Chat](../chat.md) and [Notifications and activity](../notifications-and-activity.md): where the mascot's cues show up.
- [Voice domain](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-core/src/voice/README.md): the Rust side of speech.
