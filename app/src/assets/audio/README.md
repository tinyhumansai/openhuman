# Audio assets

Short UI chimes for the push-to-talk feature (`docs/superpowers/specs/2026-06-02-global-ptt-design.md`).

| File            | Purpose                                                     | Source                                                                     | License              |
| --------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------- | -------------------- |
| `ptt-open.wav`  | Mic opened (PTT key pressed).                               | Generated locally with Python `wave` + sine generator (800–1200 Hz sweep). | CC0 / Public Domain. |
| `ptt-close.wav` | Mic closed (PTT key released).                              | Generated locally with Python `wave` + sine generator (1200–800 Hz sweep). | CC0 / Public Domain. |
| `ptt-error.wav` | Session aborted (empty audio, mic permission denied, etc.). | Generated locally with Python `wave` + sine generator (250 Hz tone).       | CC0 / Public Domain. |

All clips are ~80–120ms, LUFS-normalized to roughly match the in-app notification sound (~ -16 LUFS). Replace freely with better-sounding equivalents — just keep them under 200ms and CC0/MIT-equivalent.
