---
description: >-
  The on-screen face of OpenHuman, a desktop mascot that speaks, reacts, joins
  your meetings, and thinks in the background even when you aren't looking at
  it.
icon: face-smile
---

# The Mascot

OpenHuman has a face. The mascot is an animated character that lives on your desktop and acts as the visible surface of the agent, what it's saying, what it's thinking about, when it's idle, when it's busy, when it has something to tell you.

It is not a chrome ornament. The mascot is wired into the same pieces as the rest of the agent: voice, memory, the [subconscious loop](../subconscious.md), and the [Google Meet integration](../native-tools/voice.md). When the agent talks, the mascot is the one talking; when the agent is thinking, the mascot is the one thinking.

## What it does

### It speaks, and lip-syncs to its own voice

When the agent replies, the audio is generated through a hosted TTS model and streamed to your speakers. At the same time, the mascot drives a viseme map against the audio so its mouth shapes match the words coming out. There's no separate "talking head" video, the same audio stream that you hear is the one driving the animation.

See [Native Voice](../native-tools/voice.md) for the speech-to-text, text-to-speech, and meeting plumbing the mascot rides on top of.

### It joins your meetings, as a real participant

The mascot is OpenHuman's flagship voice integration. It can join a Google Meet call as a real participant: it hears everyone, takes notes into your [Memory Tree](../obsidian-wiki/memory-tree.md), speaks back into the call when it has something to say, and pipes its own animated face into the meeting as the camera feed.

This is the headline use case and has its own page, see [Meeting Agents](meeting-agents.md).

### It moves and reacts to its surroundings

The mascot has mood states (idle, thinking, listening, talking, surprised, dreaming) and it transitions between them based on what the agent is doing. When you start typing it shifts into a listening pose. When the model is reasoning, it shows that. When a tool call returns something noteworthy, it reacts. When you stop interacting for a while, it drifts into idle.

After a turn finishes, the desktop mascot also reads the conversation-level cue that arrives with the chat result. A success cue produces a short happy acknowledgement, uncertainty produces a confused acknowledgement, and warnings or failed outcomes produce a concerned acknowledgement. If no strong cue is present, it keeps the existing calm post-turn acknowledgement and falls back to idle.

It is meant to feel alive, not animated-on-rails.

### It remembers you

The mascot is the visible part of an agent that has the [Memory Tree](../obsidian-wiki/memory-tree.md) underneath it. It remembers what you've talked about, who the people in your life are, what's open on your plate, what's been decided, and what's outstanding, across every source you've connected. When it greets you in the morning, it isn't starting from zero.

That memory is what makes the personality consistent over weeks and months. The mascot you talk to today knows what the mascot you talked to last Tuesday knows.

### It thinks in the background, the subconscious

Even when you've stopped typing, the mascot keeps thinking. The [Subconscious Loop](../subconscious.md) is a background tick that:

* Loads your standing tasks and ambient goals.
* Reads the current state of your workspace and recent memory.
* Decides what to do about each one (execute autonomously, hold, or escalate to you for approval).
* Writes the outcome back to an activity log you can audit.

So when you come back to the desk, the mascot may have already drafted the email, refreshed the dashboard, or queued the question it needs to ask you. The face on the screen is the one that did the work.

### It dreams

When you're away long enough, the mascot enters a dreaming state. Dreaming is the agent's offline consolidation pass, distilling the day's chunks into longer-horizon summaries, refreshing topic trees for the entities that have heated up, surfacing patterns that didn't fit any single source. The mascot animates differently while dreaming so you can tell at a glance: it's not idle, it's processing.

When you come back, the dreams have already been folded into the Memory Tree. The mascot wakes up smarter than it went to sleep.

## Why have a mascot at all?

Most assistants are a blinking text input. That's fine for a tool. It's not fine for something that's meant to be alongside you all day, with persistent memory of your life, taking actions on your behalf.

The mascot exists because:

* **Presence beats panels.** A face you can glance at tells you, in one frame, whether the agent is busy, idle, dreaming, or trying to get your attention.
* **It makes voice calls feel like a conversation.** A camera feed of an animated character lip-syncing to its own speech is a different experience than a robotic voice with a black tile.
* **Personality is a UX surface.** A consistent character on screen is easier to trust, talk to, and forgive when it makes a mistake than a faceless API.

## See also

* [Meeting Agents](meeting-agents.md), the mascot in Google Meet: listening, speaking, animating, using tools.
* [Native Voice](../native-tools/voice.md), the STT / TTS plumbing the mascot rides on.
* [Memory Tree](../obsidian-wiki/memory-tree.md), what the mascot remembers, and how.
* [Subconscious Loop](../subconscious.md), what it thinks about while you're away.
* [Chromium Embedded Framework](../../developing/cef.md), the camera-into-Meet pipeline (developer reference).
