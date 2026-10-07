# Dictation audio fixture

`dictation-tone.webm` is a short synthetic 440 Hz tone recorded as WebM/Opus
using Chrome's `MediaRecorder`. A Web Audio oscillator at gain 0.1 was connected
to an `AudioContext.createMediaStreamDestination()` and stopped after 0.25 seconds.
No microphone or personal audio was used.

The fake recorder emits these valid compressed bytes so browser tests exercise
the application's real `encodeBlobToWav` path, including browser decoding and
resampling. Tests check that a rejected native request is retried with a valid,
non-silent 16 kHz mono PCM WAV and inserts only one final transcript. Both STT
responses are mocked, and external HTTP requests are blocked.
