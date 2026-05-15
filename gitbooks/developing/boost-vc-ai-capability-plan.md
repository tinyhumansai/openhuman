---
description: >-
  Roadmap for free or open OpenHuman assistant capabilities inspired by the
  Boost VC AI portfolio.
---

# Boost VC AI Capability Plan

Issue [#1830](https://github.com/tinyhumansai/openhuman/issues/1830) is an
umbrella initiative. This plan defines the selected commercial inspirations,
the OpenHuman features to replicate with free or open components, and the
explicit non-goals for the first implementation cut.

## Goals

OpenHuman should absorb the useful, desktop-assistant-shaped parts of the
current Boost VC AI portfolio without cloning closed products or adding brittle
one-off integrations. The work should strengthen the existing Rust core,
controller registry, React/Tauri shell, skills runtime, and privacy model.

The target user outcome is a stronger desktop assistant that can:

- listen and speak through free or open voice components;
- show captions and save transcripts;
- map voice intent onto approved actions;
- triage and draft inbox or outreach work;
- answer questions over local or connected data;
- guide users through recommendation or intake flows.

## Selected Inspirations

| Inspiration        | OpenHuman capability                                | Tracking issue                                                 | First reusable primitive                                                 |
| ------------------ | --------------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Deepgram           | Free voice foundation: STT, TTS, voice sessions     | [#1831](https://github.com/tinyhumansai/openhuman/issues/1831) | Local STT/TTS providers behind `openhuman.local_ai_*` and voice settings |
| Ava                | Live captions, transcripts, meeting notes           | [#1832](https://github.com/tinyhumansai/openhuman/issues/1832) | Transcript event stream and persisted transcript records                 |
| Screevo            | Voice-driven desktop and workflow actions           | [#1833](https://github.com/tinyhumansai/openhuman/issues/1833) | Voice intent mapped to controller-backed actions                         |
| Kriya / KalendarAI | Inbox triage, outreach drafting, scheduling handoff | [#1834](https://github.com/tinyhumansai/openhuman/issues/1834) | Channel-agnostic message triage and draft schema                         |
| Athenic            | Chat with data, proactive insights, anomaly notes   | [#1835](https://github.com/tinyhumansai/openhuman/issues/1835) | Dataset adapter and sourced answer envelope                              |
| Octane AI          | Guided recommendation and intake flows              | [#1836](https://github.com/tinyhumansai/openhuman/issues/1836) | Reusable guided-flow state machine                                       |

## Current Inventory

The repository already has several building blocks that should be reused before
new systems are added:

- Local speech controllers exist in `src/openhuman/local_ai/schemas.rs`:
  `local_ai_transcribe`, `local_ai_transcribe_bytes`, `local_ai_tts`,
  `local_ai_install_whisper`, and `local_ai_install_piper`.
- Speech execution lives under `src/openhuman/local_ai/service/speech.rs`, with
  Whisper and Piper install plumbing in `src/openhuman/local_ai/install_whisper.rs`
  and `src/openhuman/local_ai/install_piper.rs`.
- The app has voice settings and diagnostics in
  `app/src/components/settings/panels/VoicePanel.tsx` and
  `app/src/components/settings/panels/VoiceDebugPanel.tsx`.
- Conversation voice mode still exists in `app/src/pages/Conversations.tsx`,
  but some voice UI paths are hidden or retained for re-enable work.
- Google Meet caption plumbing exists in `app/src-tauri/src/meet_audio/` and
  account scanner support already knows about Meet captions.
- The user-facing capability catalog lives in `src/openhuman/about_app/` and
  must be updated whenever a shipped capability changes user-visible behavior.

## Architecture Rules

All phases follow these constraints:

- Core business rules live in Rust domains.
- App code owns user experience, permissions, confirmations, and display state.
- New public operations go through controller registry metadata and generic RPC
  adapters.
- Skills can orchestrate capabilities but must not become the only business
  logic home for shared product behavior.
- No feature may add a bespoke JSON-RPC branch when a controller schema can
  express it.
- Logs must be structured enough for support and E2E debugging, and must not
  leak audio, transcript, inbox, dataset, or credential contents.
- Free or open components are preferred. Hosted providers can remain optional
  fallbacks, but the default roadmap cannot depend on closed-only APIs.

## Phased Implementation

### Phase 1: Voice Foundation

Tracking: [#1831](https://github.com/tinyhumansai/openhuman/issues/1831)

First cut:

- Normalize the existing Whisper and Piper paths as the free/open defaults for
  local STT and TTS.
- Expose one documented app flow for microphone voice input into conversations.
- Keep provider choice in settings so hosted speech can remain an optional
  fallback.
- Add capability-catalog entries or updates for local STT, local TTS, and voice
  conversation input.

Non-goals:

- Real-time multi-speaker diarization.
- Full desktop command execution by voice.
- New hosted speech vendor integrations.

### Phase 2: Captions And Transcripts

Tracking: [#1832](https://github.com/tinyhumansai/openhuman/issues/1832)

First cut:

- Define a transcript record shape shared by Meet captions and microphone
  sessions.
- Persist transcripts with source, timestamps, participant labels when known,
  and summary status.
- Display live captions for one supported source before generalizing.
- Add one summary or meeting-note action on saved transcripts.

Non-goals:

- System-wide audio loopback capture on every platform.
- Perfect speaker diarization.
- Translation.

### Phase 3: Voice Actions

Tracking: [#1833](https://github.com/tinyhumansai/openhuman/issues/1833)

First cut:

- Convert recognized speech into a small intent envelope:
  utterance, candidate action, confidence, required confirmation, and source.
- Route execution through registered controllers or skill-backed commands.
- Start with safe, reversible actions such as opening a page, starting a search,
  or creating a draft.
- Require visible confirmation before destructive or externally visible actions.

Non-goals:

- Unconfirmed send/delete/purchase actions.
- Arbitrary shell execution from speech.
- Training a custom intent model.

### Phase 4: Operator Inbox Assistant

Tracking: [#1834](https://github.com/tinyhumansai/openhuman/issues/1834)

First cut:

- Pick one message source already represented in the channel stack.
- Add a triage record with source, thread id, priority, reason, proposed reply,
  and follow-up timestamp.
- Generate reply or outreach drafts, not auto-send messages.
- Handoff scheduling to an existing reminder, calendar, or task flow.

Non-goals:

- Bulk outbound campaigns.
- Sending messages without explicit review.
- Replacing channel provider auth or scanner architecture.

### Phase 5: Analytics Assistant

Tracking: [#1835](https://github.com/tinyhumansai/openhuman/issues/1835)

First cut:

- Support one local dataset source such as CSV or exported JSON.
- Return sourced answers with dataset name, columns used, filters, and caveats.
- Add one proactive insight job, such as simple anomaly or trend detection.
- Show provenance in the app instead of raw opaque model text.

Non-goals:

- Full BI dashboard replacement.
- Unbounded SQL execution against production databases.
- Silent remote upload of user datasets.

### Phase 6: Guided Recommendation Flows

Tracking: [#1836](https://github.com/tinyhumansai/openhuman/issues/1836)

First cut:

- Define reusable guided-flow state: prompt, answer type, validation, branching,
  recommendation, and completion event.
- Ship one concrete flow, such as onboarding setup guidance or tool selection.
- Keep recommendation rules in a Rust domain or reusable skill contract, not
  inside a single React component.

Non-goals:

- A visual no-code flow builder.
- Ecommerce-specific personalization.
- Hidden persistence of sensitive answers.

## Validation Requirements

Each implementation PR should include the smallest relevant slice of:

- Rust unit tests for domain rules, schemas, and validation.
- JSON-RPC/controller tests for exposed operations.
- App unit tests for settings, panels, confirmations, and error states.
- Targeted app E2E coverage for the user path.
- Capability catalog updates in `src/openhuman/about_app/` when behavior becomes
  user-facing.
- Debug logging across core, shell, and app layers.
- Diff coverage >= 80 percent.

## PR Boundaries

Do not try to close [#1830](https://github.com/tinyhumansai/openhuman/issues/1830)
with one mega-PR. The expected sequence is:

1. Land this capability plan.
2. Ship [#1831](https://github.com/tinyhumansai/openhuman/issues/1831) as the
   first code PR because existing local STT/TTS primitives reduce scope.
3. Ship [#1832](https://github.com/tinyhumansai/openhuman/issues/1832) after
   voice input and transcript primitives are stable.
4. Split [#1833](https://github.com/tinyhumansai/openhuman/issues/1833) through
   [#1836](https://github.com/tinyhumansai/openhuman/issues/1836) by domain and
   avoid overlapping UI or controller surfaces.
