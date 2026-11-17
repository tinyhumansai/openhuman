# Voice domain

The voice domain provides speech-to-text, text-to-speech, provider settings,
reply speech, and live voice sessions. Hosted transcription providers are
selected through the configured STT engine; TinyVoice supplies the transcript
hallucination check used by conversation transcription.

The `voice` Cargo feature is off in contributor defaults and enabled for the
desktop product. Its disabled facade keeps the shared provider types available
to builds that omit voice. Live voice and the other provider controllers are
registered through the core controller registry.

The retained modules are:

- `factory/`: configured speech provider selection.
- `live/`: real-time voice sessions and their RPC controllers.
- `realtime_harness/`: the live-session test harness.
- `ops.rs`: voice availability, transcription, synthesis, and provider updates.
- `postprocess.rs`: transcript cleanup.
- `local_speech.rs`: local speech utilities.
- `types.rs`: voice domain result types.
- `compile_status.rs`: compile-time feature status.

Module behavior remains behind the TinyVoice bus contract. OpenHuman owns the
product policy and uses the module for transcript screening.
