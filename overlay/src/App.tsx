import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useMemo, useRef, useState } from "react";

const TARGET_SAMPLE_RATE = 16000;

type OverlayStatus = "idle" | "listening" | "transcribing" | "ready" | "error";

interface TranscribeResult {
  text: string;
  raw_text: string;
  model_id: string;
}

function logOverlay(message: string, details?: Record<string, unknown>) {
  if (details) {
    console.debug(`[overlay] ${message}`, details);
    return;
  }
  console.debug(`[overlay] ${message}`);
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

  const [status, setStatus] = useState<OverlayStatus>("idle");
  const [message, setMessage] = useState("Click to start listening");
  const [transcript, setTranscript] = useState("");

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
        const result = await invoke<TranscribeResult>("core_rpc", {
          method: "openhuman.voice_transcribe_bytes",
          params: {
            audio_bytes: audioBytes,
            extension: "wav",
            skip_cleanup: false,
          },
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
    [insertTranscriptIntoFocusedField],
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
    if (status === "listening") {
      logOverlay("main button toggled to stop listening");
      stopRecording();
      return;
    }

    logOverlay("main button toggled to start listening", { priorStatus: status });
    void startRecording();
  }, [startRecording, status, stopRecording]);

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
          className={`relative w-[148px] rounded-[40px] border border-white/15 bg-gradient-to-br p-3 backdrop-blur-xl transition-all duration-200 ${shellClassName}`}
          onMouseDown={(event) => {
            if (event.target instanceof HTMLElement && event.target.closest("button")) {
              return;
            }
            void appWindow.startDragging();
          }}
        >
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

          <button
            type="button"
            onClick={handleMainButton}
            className={`group relative flex h-[108px] w-full items-center justify-center rounded-full border border-white/20 bg-black/20 transition duration-200 hover:bg-black/28 ${
              status === "listening" ? "scale-[1.02]" : ""
            }`}
            aria-label={status === "listening" ? "Stop listening" : "Start listening"}
          >
            <span className="absolute inset-3 rounded-full border border-white/12" />
            <MicrophoneIcon active={status === "listening"} />
          </button>

          <div className="mt-3 text-center">
            <p className="text-[11px] font-semibold uppercase tracking-[0.18em] opacity-80">
              {status}
            </p>
            <p className="mt-1 text-xs leading-4 opacity-90">{message}</p>
          </div>

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
    </div>
  );
}
