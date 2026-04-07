import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { callParentCoreRpc } from "./parentCoreRpc";

const TARGET_SAMPLE_RATE = 16000;

type OverlayStatus = "idle" | "listening" | "transcribing" | "ready" | "error";

interface TranscribeResult {
  text: string;
  raw_text: string;
  model_id: string;
}

interface GlobeHotkeyStatus {
  supported: boolean;
  running: boolean;
  input_monitoring_permission: string;
  last_error: string | null;
  events_pending: number;
}

interface GlobeHotkeyPollResult {
  status: GlobeHotkeyStatus;
  events: string[];
}

interface AppContextInfo {
  app_name: string | null;
  window_title: string | null;
}

interface AccessibilitySessionStatus {
  active: boolean;
  capture_count: number;
  frames_in_memory: number;
  last_capture_at_ms: number | null;
  last_context: string | null;
  last_window_title: string | null;
  vision_enabled: boolean;
  vision_state: string;
  vision_queue_depth: number;
}

interface AccessibilityStatus {
  is_context_blocked: boolean;
  foreground_context: AppContextInfo | null;
  session: AccessibilitySessionStatus;
}

interface AutocompleteSuggestion {
  value: string;
  confidence: number;
}

interface AutocompleteStatus {
  platform_supported: boolean;
  enabled: boolean;
  running: boolean;
  phase: string;
  app_name: string | null;
  last_error: string | null;
  updated_at_ms: number | null;
  suggestion: AutocompleteSuggestion | null;
}

/** Matches `VoiceStatus` in src/openhuman/voice/types.rs */
interface VoiceStatus {
  stt_available: boolean;
  tts_available: boolean;
  stt_model_id: string;
  tts_voice_id: string;
  whisper_binary: string | null;
  piper_binary: string | null;
  stt_model_path: string | null;
  tts_voice_path: string | null;
  whisper_in_process: boolean;
  llm_cleanup_enabled: boolean;
}

interface OverlayDebugSnapshot {
  screen: AccessibilityStatus | null;
  autocomplete: AutocompleteStatus | null;
  voice: VoiceStatus | null;
  updatedAt: number | null;
  error: string | null;
}

const DEBUG_EXPANDED_KEY = "openhuman_overlay_debug_expanded";

function logOverlay(message: string, details?: unknown) {
  if (details) {
    console.debug(`[overlay] ${message}`, details);
    return;
  }
  console.debug(`[overlay] ${message}`);
}

function formatTimestamp(timestampMs: number | null): string {
  if (!timestampMs) {
    return "none";
  }

  try {
    return new Date(timestampMs).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return String(timestampMs);
  }
}

function floatTo16BitPCM(output: DataView, offset: number, input: Float32Array) {
  for (let i = 0; i < input.length; i += 1, offset += 2) {
    const sample = Math.max(-1, Math.min(1, input[i]));
    output.setInt16(offset, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
  }
}

function encodeWavMono16k(samples: Float32Array, sampleRate: number): Uint8Array {
  const bytesPerSample = 2;
  const blockAlign = bytesPerSample;
  const byteRate = sampleRate * blockAlign;
  const dataSize = samples.length * bytesPerSample;
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  const writeString = (offset: number, value: string) => {
    for (let i = 0; i < value.length; i += 1) {
      view.setUint8(offset + i, value.charCodeAt(i));
    }
  };

  writeString(0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeString(8, "WAVE");
  writeString(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, byteRate, true);
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, 16, true);
  writeString(36, "data");
  view.setUint32(40, dataSize, true);
  floatTo16BitPCM(view, 44, samples);

  return new Uint8Array(buffer);
}

async function toMono16k(audioBuffer: AudioBuffer): Promise<Float32Array> {
  const channels = audioBuffer.numberOfChannels;
  const mono = new Float32Array(audioBuffer.length);

  for (let c = 0; c < channels; c += 1) {
    const channelData = audioBuffer.getChannelData(c);
    for (let i = 0; i < audioBuffer.length; i += 1) {
      mono[i] += channelData[i] / channels;
    }
  }

  if (audioBuffer.sampleRate === TARGET_SAMPLE_RATE) {
    return mono;
  }

  const targetLength = Math.max(
    1,
    Math.round((mono.length * TARGET_SAMPLE_RATE) / audioBuffer.sampleRate),
  );
  const offline = new OfflineAudioContext(1, targetLength, TARGET_SAMPLE_RATE);
  const sourceBuffer = offline.createBuffer(1, mono.length, audioBuffer.sampleRate);
  sourceBuffer.copyToChannel(mono, 0);
  const source = offline.createBufferSource();
  source.buffer = sourceBuffer;
  source.connect(offline.destination);
  source.start();
  const rendered = await offline.startRendering();
  return rendered.getChannelData(0).slice();
}

async function convertBlobToWavBytes(blob: Blob): Promise<number[]> {
  const arrayBuffer = await blob.arrayBuffer();
  const audioContext = new AudioContext();

  try {
    const decoded = await audioContext.decodeAudioData(arrayBuffer.slice(0));
    const mono16k = await toMono16k(decoded);
    return Array.from(encodeWavMono16k(mono16k, TARGET_SAMPLE_RATE));
  } finally {
    await audioContext.close();
  }
}

function MicrophoneIcon({ active }: { active: boolean }) {
  return (
    <svg
      aria-hidden="true"
      className={`h-9 w-9 transition-transform duration-200 ${active ? "scale-105" : ""}`}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M6 11a6 6 0 0 0 12 0" />
      <path d="M12 17v4" />
      <path d="M8.5 21h7" />
    </svg>
  );
}

export function App() {
  const appWindow = getCurrentWindow();
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const sessionIdRef = useRef(0);
  const globePollInFlightRef = useRef(false);

  /** `undefined` until Tauri reports env; then URL string or `null` (embedded core only). */
  const [parentRpcUrl, setParentRpcUrl] = useState<string | null | undefined>(undefined);
  const [coreReachable, setCoreReachable] = useState(true);
  const [voiceCaptureEnabled, setVoiceCaptureEnabled] = useState(true);
  const [debugExpanded, setDebugExpanded] = useState(() => {
    try {
      return typeof localStorage !== "undefined" && localStorage.getItem(DEBUG_EXPANDED_KEY) === "1";
    } catch {
      return false;
    }
  });

  const [status, setStatus] = useState<OverlayStatus>("idle");
  const [message, setMessage] = useState("Click to start listening");
  const [transcript, setTranscript] = useState("");
  const [debugSnapshot, setDebugSnapshot] = useState<OverlayDebugSnapshot>({
    screen: null,
    autocomplete: null,
    voice: null,
    updatedAt: null,
    error: null,
  });

  useEffect(() => {
    let mounted = true;
    void invoke<string | null>("overlay_parent_rpc_url")
      .then((url) => {
        if (!mounted) return;
        const trimmed = url?.trim();
        setParentRpcUrl(trimmed && trimmed.length > 0 ? trimmed : null);
      })
      .catch(() => {
        if (mounted) setParentRpcUrl(null);
      });
    return () => {
      mounted = false;
    };
  }, []);

  const rpc = useCallback(
    async <T,>(method: string, params: Record<string, unknown> = {}): Promise<T> => {
      if (parentRpcUrl === undefined) {
        throw new Error("[overlay] RPC not initialized");
      }
      if (parentRpcUrl) {
        return callParentCoreRpc<T>(parentRpcUrl, method, params);
      }
      return invoke<T>("core_rpc", { method, params });
    },
    [parentRpcUrl],
  );

  const persistDebugExpanded = useCallback((expanded: boolean) => {
    setDebugExpanded(expanded);
    try {
      localStorage.setItem(DEBUG_EXPANDED_KEY, expanded ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    let disposed = false;

    const showOverlayFallback = async (message: string) => {
      if (disposed) {
        return;
      }
      logOverlay("globe listener unavailable", { message });
      setMessage(message);
      await appWindow.show().catch(() => {});
    };

    const startGlobeListener = async () => {
      if (parentRpcUrl === undefined) {
        return;
      }
      try {
        const result = await rpc<GlobeHotkeyStatus>("openhuman.screen_intelligence_globe_listener_start", {});
        logOverlay("globe listener start result", result);

        if (!result.supported) {
          await showOverlayFallback("Globe/Fn hotkey is only supported on macOS");
          return;
        }

        if (!result.running) {
          await showOverlayFallback(
            result.last_error ?? "Globe/Fn listener could not start. Check Input Monitoring.",
          );
        }
      } catch (error) {
        console.error("[overlay] failed to start globe listener", error);
        await showOverlayFallback("Failed to start Globe/Fn listener");
      }
    };

    const pollGlobeListener = async () => {
      if (disposed || parentRpcUrl === undefined || globePollInFlightRef.current) {
        return;
      }
      globePollInFlightRef.current = true;

      try {
        const result = await rpc<GlobeHotkeyPollResult>("openhuman.screen_intelligence_globe_listener_poll", {});

        if (disposed) {
          return;
        }

        if (!result.status.running && result.status.last_error) {
          setMessage(result.status.last_error);
        }

        if (result.events.includes("FN_UP")) {
          const visible = await appWindow.isVisible();
          logOverlay("received FN_UP", { visible });
          if (visible) {
            await appWindow.hide();
          } else {
            await appWindow.show();
          }
        }
      } catch (error) {
        if (!disposed) {
          console.warn("[overlay] globe listener poll failed", error);
        }
      } finally {
        globePollInFlightRef.current = false;
      }
    };

    void startGlobeListener();
    const intervalId = window.setInterval(() => {
      void pollGlobeListener();
    }, 175);

    return () => {
      disposed = true;
      window.clearInterval(intervalId);
      if (parentRpcUrl === undefined) {
        return;
      }
      void rpc("openhuman.screen_intelligence_globe_listener_stop", {}).catch(() => {});
    };
  }, [appWindow, parentRpcUrl, rpc]);

  useEffect(() => {
    let disposed = false;
    let pollInFlight = false;

    const pollDebugState = async () => {
      if (disposed || parentRpcUrl === undefined || pollInFlight) {
        return;
      }
      pollInFlight = true;

      try {
        if (parentRpcUrl) {
          try {
            await rpc<{ ok?: boolean }>("core.ping", {});
            if (!disposed) {
              setCoreReachable(true);
            }
          } catch {
            if (!disposed) {
              setCoreReachable(false);
            }
          }
        } else {
          setCoreReachable(true);
        }

        const [screen, autocomplete, voice] = await Promise.all([
          rpc<AccessibilityStatus>("openhuman.screen_intelligence_status", {}),
          rpc<AutocompleteStatus>("openhuman.autocomplete_status", {}),
          rpc<VoiceStatus>("openhuman.voice_status", {}),
        ]);

        if (disposed) {
          return;
        }

        logOverlay("debug snapshot refreshed", {
          screenActive: screen.session.active,
          captureCount: screen.session.capture_count,
          autocompletePhase: autocomplete.phase,
          hasSuggestion: Boolean(autocomplete.suggestion?.value),
          sttAvailable: voice.stt_available,
        });

        setDebugSnapshot({
          screen,
          autocomplete,
          voice,
          updatedAt: Date.now(),
          error: null,
        });
      } catch (error) {
        if (disposed) {
          return;
        }

        const nextError =
          error instanceof Error ? error.message : "Failed to refresh overlay debug state";
        console.warn("[overlay] debug snapshot poll failed", error);
        setDebugSnapshot((previous) => ({
          ...previous,
          updatedAt: Date.now(),
          error: nextError,
        }));
      } finally {
        pollInFlight = false;
      }
    };

    void pollDebugState();
    const intervalId = window.setInterval(() => {
      void pollDebugState();
    }, 900);

    return () => {
      disposed = true;
      window.clearInterval(intervalId);
    };
  }, [parentRpcUrl, rpc]);

  const insertTranscriptIntoFocusedField = useCallback(
    async (text: string) => {
      logOverlay("inserting transcript into focused field", { length: text.length });
      await appWindow.hide();
      await new Promise((resolve) => window.setTimeout(resolve, 120));

      try {
        await invoke("insert_text_into_focused_field", { text });
        logOverlay("transcript inserted via accessibility helper");
      } catch (error) {
        console.warn("[overlay] accessibility insert failed, falling back to clipboard", error);
        await navigator.clipboard.writeText(text);
      }
    },
    [appWindow],
  );

  const resetForNextCapture = useCallback(() => {
    setTranscript("");
    setMessage("Click to start listening");
    setStatus("idle");
  }, []);

  const cleanupStream = useCallback(() => {
    streamRef.current?.getTracks().forEach((track) => track.stop());
    streamRef.current = null;
  }, []);

  const transcribeBlob = useCallback(
    async (blob: Blob, sessionId: number) => {
      try {
        const audioBytes = await convertBlobToWavBytes(blob);
        const result = await rpc<TranscribeResult>("openhuman.voice_transcribe_bytes", {
          audio_bytes: audioBytes,
          extension: "wav",
          skip_cleanup: false,
        });

        if (sessionIdRef.current !== sessionId) {
          return;
        }

        const nextTranscript = result.text.trim();
        if (!nextTranscript) {
          setTranscript("");
          setStatus("error");
          setMessage("No speech detected");
          return;
        }

        setTranscript(nextTranscript);
        setStatus("ready");
        setMessage("Inserting text...");
        await insertTranscriptIntoFocusedField(nextTranscript);
        if (sessionIdRef.current !== sessionId) {
          return;
        }
        setMessage("Inserted into active field");
      } catch (error) {
        if (sessionIdRef.current !== sessionId) {
          return;
        }

        console.error("[overlay] transcription failed", error);
        setTranscript("");
        setStatus("error");
        setMessage(error instanceof Error ? error.message : "Transcription failed");
      }
    },
    [insertTranscriptIntoFocusedField, rpc],
  );

  const stopRecording = useCallback(() => {
    if (!mediaRecorderRef.current || mediaRecorderRef.current.state === "inactive") {
      return;
    }

    setStatus("transcribing");
    setMessage("Transcribing...");
    mediaRecorderRef.current.stop();
    mediaRecorderRef.current = null;
  }, []);

  const startRecording = useCallback(async () => {
    const nextSessionId = sessionIdRef.current + 1;
    sessionIdRef.current = nextSessionId;
    setTranscript("");
    setStatus("listening");
    setMessage("Listening...");
    chunksRef.current = [];

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;

      const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
        ? "audio/webm;codecs=opus"
        : MediaRecorder.isTypeSupported("audio/webm")
          ? "audio/webm"
          : "audio/ogg";

      const recorder = new MediaRecorder(stream, { mimeType });
      mediaRecorderRef.current = recorder;

      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          chunksRef.current.push(event.data);
        }
      };

      recorder.onerror = (event) => {
        console.error("[overlay] media recorder error", event);
        cleanupStream();
        setStatus("error");
        setMessage("Microphone recording failed");
      };

      recorder.onstop = () => {
        cleanupStream();
        const blob = new Blob(chunksRef.current, { type: mimeType });
        chunksRef.current = [];

        if (blob.size === 0) {
          setStatus("error");
          setMessage("No audio recorded");
          return;
        }

        logOverlay("recording stopped, starting transcription", {
          blobSize: blob.size,
          mimeType,
        });
        void transcribeBlob(blob, nextSessionId);
      };

      logOverlay("recording started", { mimeType });
      recorder.start(100);
    } catch (error) {
      console.error("[overlay] getUserMedia failed", error);
      cleanupStream();
      setStatus("error");
      setMessage(error instanceof Error ? error.message : "Microphone access failed");
    }
  }, [cleanupStream, transcribeBlob]);

  const handleMainButton = useCallback(() => {
    // Always allow stopping an active recording, regardless of config state.
    if (status === "listening") {
      logOverlay("main button toggled to stop listening");
      stopRecording();
      return;
    }

    if (!voiceCaptureEnabled) {
      setMessage("Turn on voice capture to use the microphone");
      return;
    }
    if (debugSnapshot.voice && !debugSnapshot.voice.stt_available) {
      setMessage(
        "Speech-to-text is not available. Configure Local AI / voice in the main OpenHuman app.",
      );
      return;
    }

    logOverlay("main button toggled to start listening", { priorStatus: status });
    void startRecording();
  }, [
    debugSnapshot.voice,
    startRecording,
    status,
    stopRecording,
    voiceCaptureEnabled,
  ]);

  const shellClassName = useMemo(() => {
    if (status === "listening") {
      return "from-red-500/90 via-rose-500/80 to-orange-400/85 text-white shadow-[0_0_64px_rgba(248,113,113,0.38)]";
    }
    if (status === "transcribing") {
      return "from-amber-400/90 via-orange-400/80 to-yellow-300/80 text-stone-950 shadow-[0_0_56px_rgba(251,191,36,0.34)]";
    }
    if (status === "error") {
      return "from-red-600/90 via-rose-700/80 to-stone-900/90 text-white shadow-[0_0_56px_rgba(190,24,93,0.35)]";
    }
    if (status === "ready") {
      return "from-emerald-400/90 via-teal-400/80 to-cyan-300/80 text-stone-950 shadow-[0_0_56px_rgba(45,212,191,0.34)]";
    }
    return "from-slate-900/92 via-slate-800/92 to-slate-700/92 text-white shadow-[0_0_48px_rgba(15,23,42,0.42)]";
  }, [status]);

  const activeScreenApp =
    debugSnapshot.screen?.foreground_context?.app_name ??
    debugSnapshot.screen?.session.last_context ??
    "Unknown app";
  const activeScreenWindow =
    debugSnapshot.screen?.foreground_context?.window_title ??
    debugSnapshot.screen?.session.last_window_title ??
    "No active window title";
  const autocompleteSuggestion = debugSnapshot.autocomplete?.suggestion?.value?.trim() ?? "";
  const autocompletePhase = debugSnapshot.autocomplete?.phase ?? "unknown";
  const autocompleteRunning =
    debugSnapshot.autocomplete?.running && debugSnapshot.autocomplete?.enabled;

  const sttAvailable = debugSnapshot.voice?.stt_available ?? true;
  const voiceBlocked =
    !voiceCaptureEnabled || (debugSnapshot.voice !== null && !debugSnapshot.voice.stt_available);
  const waitingForCoreConfig = parentRpcUrl === undefined;

  return (
    <div className="flex h-screen w-screen items-start justify-start bg-transparent p-3">
      <div className="relative select-none">
        {status === "listening" ? (
          <>
            <span className="pointer-events-none absolute inset-0 rounded-full border border-white/15 animate-ping" />
            <span className="pointer-events-none absolute -inset-3 rounded-full border border-red-300/30 blur-[2px]" />
          </>
        ) : null}

        <div
          className={`relative w-[348px] rounded-[32px] border border-white/15 bg-gradient-to-br p-3 backdrop-blur-xl transition-all duration-200 ${shellClassName}`}
          onMouseDown={(event) => {
            if (event.target instanceof HTMLElement && event.target.closest("button")) {
              return;
            }
            void appWindow.startDragging();
          }}
        >
          {parentRpcUrl && !coreReachable ? (
            <div className="mb-2 rounded-2xl border border-amber-400/35 bg-amber-950/35 px-3 py-2 text-[11px] leading-4 text-amber-50">
              Cannot reach the OpenHuman core at the sidecar URL. Autocomplete and screen debug may
              be stale. Check that the main app is running.
            </div>
          ) : null}

          <div className="mb-3 flex items-center justify-between gap-2">
            <span className="rounded-full bg-black/15 px-2 py-1 text-[10px] font-semibold uppercase tracking-[0.24em]">
              Voice
            </span>
            <button
              type="button"
              className="h-7 w-7 rounded-full border border-white/15 bg-black/20 text-sm transition hover:bg-black/30"
              onClick={() => appWindow.hide()}
              aria-label="Hide overlay"
            >
              ×
            </button>
          </div>

          <div className="mb-3 flex items-center justify-between gap-2 rounded-2xl border border-white/10 bg-black/15 px-3 py-2">
            <div className="min-w-0">
              <p className="text-[10px] font-semibold uppercase tracking-[0.18em] opacity-80">
                Voice capture
              </p>
              <p className="mt-0.5 text-[11px] leading-4 opacity-85">
                {waitingForCoreConfig
                  ? "Checking core…"
                  : sttAvailable
                    ? debugSnapshot.voice?.whisper_in_process
                      ? "STT ready (in-process)"
                      : "STT ready"
                    : "STT unavailable — configure voice in the main app"}
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={voiceCaptureEnabled}
              aria-label={voiceCaptureEnabled ? "Turn voice capture off" : "Turn voice capture on"}
              className={`relative h-8 w-[52px] shrink-0 rounded-full border border-white/15 transition ${
                voiceCaptureEnabled ? "bg-emerald-500/50" : "bg-black/35"
              }`}
              onClick={() => setVoiceCaptureEnabled((previous) => !previous)}
            >
              <span
                className={`absolute top-1 h-6 w-6 rounded-full bg-white shadow transition ${
                  voiceCaptureEnabled ? "left-7" : "left-1"
                }`}
              />
            </button>
          </div>

          <div className="flex items-start gap-3">
            <button
              type="button"
              onClick={handleMainButton}
              disabled={waitingForCoreConfig || voiceBlocked}
              className={`group relative flex h-[108px] w-[108px] shrink-0 items-center justify-center rounded-full border border-white/20 bg-black/20 transition duration-200 hover:bg-black/28 disabled:cursor-not-allowed disabled:opacity-40 ${
                status === "listening" ? "scale-[1.02]" : ""
              }`}
              aria-label={status === "listening" ? "Stop listening" : "Start listening"}
            >
              <span className="absolute inset-3 rounded-full border border-white/12" />
              <MicrophoneIcon active={status === "listening"} />
            </button>

            <div className="min-w-0 flex-1 pt-1">
              <p className="text-[11px] font-semibold uppercase tracking-[0.18em] opacity-80">
                {status}
              </p>
              <p className="mt-1 text-xs leading-4 opacity-90">{message}</p>

              {transcript ? (
                <div className="mt-3 rounded-2xl border border-white/10 bg-black/15 px-3 py-2 text-[11px] leading-4 opacity-95">
                  {transcript}
                </div>
              ) : null}

              {(status === "ready" || status === "error") && !transcript ? (
                <button
                  type="button"
                  className="mt-3 w-full rounded-full border border-white/12 bg-black/15 px-3 py-2 text-[11px] font-medium transition hover:bg-black/25"
                  onClick={resetForNextCapture}
                >
                  Reset
                </button>
              ) : null}
            </div>
          </div>

          <div className="mt-3 rounded-[24px] border border-white/10 bg-black/15 p-3">
            <div className="flex items-center justify-between gap-2">
              <button
                type="button"
                className="flex flex-1 items-center justify-between gap-2 text-left"
                onClick={() => persistDebugExpanded(!debugExpanded)}
                aria-expanded={debugExpanded}
              >
                <span className="text-[10px] font-semibold uppercase tracking-[0.22em] opacity-75">
                  Debug {debugExpanded ? "▼" : "▶"}
                </span>
                <span className="text-[10px] opacity-65">
                  {debugSnapshot.updatedAt ? formatTimestamp(debugSnapshot.updatedAt) : "waiting"}
                </span>
              </button>
            </div>

            {!debugExpanded ? (
              <p className="mt-2 text-[11px] leading-4 opacity-80">
                Screen: {debugSnapshot.screen?.session.active ? "session on" : "idle"} ·
                Autocomplete: {autocompletePhase}
                {debugSnapshot.autocomplete?.last_error ? " · error" : ""}
              </p>
            ) : null}

            {debugSnapshot.error ? (
              <div className="mt-3 rounded-2xl border border-red-300/20 bg-red-950/20 px-3 py-2 text-[11px] leading-4 text-red-100">
                {debugSnapshot.error}
              </div>
            ) : null}

            {debugExpanded ? (
            <div className="mt-3 grid gap-3">
              <section className="rounded-2xl border border-white/8 bg-black/10 px-3 py-2">
                <div className="text-[10px] font-semibold uppercase tracking-[0.18em] opacity-70">
                  Voice / STT
                </div>
                <div className="mt-2 space-y-1 text-[11px] leading-4 opacity-90">
                  <p>STT: {sttAvailable ? "available" : "unavailable"}</p>
                  <p className="truncate">Model: {debugSnapshot.voice?.stt_model_id ?? "—"}</p>
                  <p className="truncate">
                    Whisper: {debugSnapshot.voice?.whisper_binary ?? "not found"}
                  </p>
                </div>
              </section>

              <section className="rounded-2xl border border-white/8 bg-black/10 px-3 py-2">
                <div className="text-[10px] font-semibold uppercase tracking-[0.18em] opacity-70">
                  Screen Intelligence
                </div>
                <div className="mt-2 space-y-1 text-[11px] leading-4 opacity-90">
                  <p>Active screen: {activeScreenApp}</p>
                  <p className="truncate">Window: {activeScreenWindow}</p>
                  <p>
                    Screenshots: {debugSnapshot.screen?.session.capture_count ?? 0} total,{" "}
                    {debugSnapshot.screen?.session.frames_in_memory ?? 0} in memory
                  </p>
                  <p>
                    Session: {debugSnapshot.screen?.session.active ? "active" : "idle"} | Vision:{" "}
                    {debugSnapshot.screen?.session.vision_enabled ? "on" : "off"} /{" "}
                    {debugSnapshot.screen?.session.vision_state ?? "idle"}
                  </p>
                  <p>
                    Queue: {debugSnapshot.screen?.session.vision_queue_depth ?? 0} | Blocked:{" "}
                    {debugSnapshot.screen?.is_context_blocked ? "yes" : "no"}
                  </p>
                  <p>
                    Last capture:{" "}
                    {formatTimestamp(debugSnapshot.screen?.session.last_capture_at_ms ?? null)}
                  </p>
                </div>
              </section>

              <section className="rounded-2xl border border-white/8 bg-black/10 px-3 py-2">
                <div className="text-[10px] font-semibold uppercase tracking-[0.18em] opacity-70">
                  Autocomplete
                </div>
                <div className="mt-2 space-y-1 text-[11px] leading-4 opacity-90">
                  <p>
                    Status: {autocompleteRunning ? "active" : "idle"} | Phase: {autocompletePhase}
                  </p>
                  <p>App: {debugSnapshot.autocomplete?.app_name ?? activeScreenApp}</p>
                  <p>
                    Processing:{" "}
                    {autocompletePhase === "refreshing" || autocompletePhase === "processing"
                      ? "yes"
                      : "no"}
                  </p>
                  <p>
                    Suggestions:{" "}
                    {autocompleteSuggestion ? "1 ready" : "none"}
                  </p>
                  <div className="rounded-xl border border-white/8 bg-black/10 px-2 py-2 text-[11px] leading-4">
                    {autocompleteSuggestion || "No autocomplete suggestion available."}
                  </div>
                  {debugSnapshot.autocomplete?.last_error ? (
                    <p className="text-red-100">
                      Error: {debugSnapshot.autocomplete.last_error}
                    </p>
                  ) : null}
                </div>
              </section>
            </div>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
