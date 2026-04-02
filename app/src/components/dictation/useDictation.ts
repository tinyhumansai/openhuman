import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../../services/coreRpcClient';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { resetDictation, setError, setStatus, setTranscript } from '../../store/dictationSlice';
import { registerDictationHotkey } from '../../utils/tauriCommands';

const TARGET_SAMPLE_RATE = 16000;

interface TranscribeResult {
  text: string;
  raw_text: string;
  model_id: string;
}

interface ParsedHotkey {
  key: string;
  cmdOrCtrl: boolean;
  cmd: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  super: boolean;
}

function parseHotkey(shortcut: string): ParsedHotkey | null {
  const tokens = shortcut
    .split('+')
    .map(t => t.trim().toLowerCase())
    .filter(Boolean);
  if (tokens.length === 0) return null;

  const parsed: ParsedHotkey = {
    key: '',
    cmdOrCtrl: false,
    cmd: false,
    ctrl: false,
    shift: false,
    alt: false,
    super: false,
  };

  for (const token of tokens) {
    switch (token) {
      case 'cmdorctrl':
      case 'commandorcontrol':
        parsed.cmdOrCtrl = true;
        break;
      case 'cmd':
      case 'command':
      case 'meta':
        parsed.cmd = true;
        break;
      case 'ctrl':
      case 'control':
        parsed.ctrl = true;
        break;
      case 'shift':
        parsed.shift = true;
        break;
      case 'alt':
      case 'option':
        parsed.alt = true;
        break;
      case 'super':
        parsed.super = true;
        break;
      default:
        parsed.key = token;
        break;
    }
  }

  return parsed.key ? parsed : null;
}

function matchesHotkeyEvent(event: KeyboardEvent, parsed: ParsedHotkey): boolean {
  const key = event.key.toLowerCase();
  if (key !== parsed.key.toLowerCase()) return false;
  if (parsed.shift !== event.shiftKey) return false;
  if (parsed.alt !== event.altKey) return false;

  if (parsed.cmdOrCtrl && !(event.metaKey || event.ctrlKey)) return false;
  if (parsed.cmd && !event.metaKey) return false;
  if (parsed.ctrl && !event.ctrlKey) return false;
  if (parsed.super && !event.metaKey) return false;

  return true;
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

  writeString(0, 'RIFF');
  view.setUint32(4, 36 + dataSize, true);
  writeString(8, 'WAVE');
  writeString(12, 'fmt ');
  view.setUint32(16, 16, true); // PCM header size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, byteRate, true);
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, 16, true); // 16-bit
  writeString(36, 'data');
  view.setUint32(40, dataSize, true);
  floatTo16BitPCM(view, 44, samples);

  return new Uint8Array(buffer);
}

async function toMono16k(audioBuffer: AudioBuffer): Promise<Float32Array> {
  const channels = audioBuffer.numberOfChannels;
  const inputLength = audioBuffer.length;

  // Downmix to mono by averaging channels.
  const mono = new Float32Array(inputLength);
  for (let c = 0; c < channels; c += 1) {
    const channelData = audioBuffer.getChannelData(c);
    for (let i = 0; i < inputLength; i += 1) {
      mono[i] += channelData[i] / channels;
    }
  }

  if (audioBuffer.sampleRate === TARGET_SAMPLE_RATE) {
    return mono;
  }

  // Resample to 16k via OfflineAudioContext for whisper-rs compatibility.
  const targetLength = Math.max(
    1,
    Math.round((mono.length * TARGET_SAMPLE_RATE) / audioBuffer.sampleRate)
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
    const wav = encodeWavMono16k(mono16k, TARGET_SAMPLE_RATE);
    console.debug(
      '[dictation] converted audio to wav bytes=%d sampleRate=%d',
      wav.length,
      TARGET_SAMPLE_RATE
    );
    return Array.from(wav);
  } finally {
    await audioContext.close();
  }
}

export function useDictation() {
  const dispatch = useAppDispatch();
  const { status, hotkey } = useAppSelector(s => s.dictation);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const toggleRef = useRef<() => void>(() => {});
  const sessionIdRef = useRef(0);
  const [isSupported, setIsSupported] = useState(false);

  useEffect(() => {
    setIsSupported(
      typeof navigator !== 'undefined' &&
        'mediaDevices' in navigator &&
        'getUserMedia' in navigator.mediaDevices
    );
  }, []);

  const startRecording = useCallback(async () => {
    if (status === 'recording') return;
    const sessionId = sessionIdRef.current + 1;
    sessionIdRef.current = sessionId;
    dispatch(setStatus('recording'));
    dispatch(setError(null));
    chunksRef.current = [];

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : MediaRecorder.isTypeSupported('audio/webm')
          ? 'audio/webm'
          : 'audio/ogg';

      const recorder = new MediaRecorder(stream, { mimeType });
      mediaRecorderRef.current = recorder;

      recorder.ondataavailable = e => {
        if (e.data.size > 0) {
          chunksRef.current.push(e.data);
        }
      };

      recorder.onstop = async () => {
        // Release mic
        stream.getTracks().forEach(t => t.stop());
        if (sessionIdRef.current !== sessionId) {
          console.debug('[dictation] ignoring stale onstop callback session=%d', sessionId);
          return;
        }

        const blob = new Blob(chunksRef.current, { type: mimeType });
        if (blob.size === 0) {
          dispatch(setError('No audio recorded'));
          return;
        }

        dispatch(setStatus('transcribing'));
        console.debug('[dictation] transcribing blob size=%d mimeType=%s', blob.size, mimeType);

        try {
          let bytes: number[];
          let ext: string;
          try {
            bytes = await convertBlobToWavBytes(blob);
            ext = 'wav';
          } catch (conversionErr) {
            // Fallback for environments where decode/resample is unavailable.
            console.warn(
              '[dictation] wav conversion failed, falling back to raw blob path',
              conversionErr
            );
            const buffer = await blob.arrayBuffer();
            bytes = Array.from(new Uint8Array(buffer));
            ext = mimeType.includes('ogg') ? 'ogg' : 'webm';
          }

          console.debug('[dictation] calling voice_transcribe_bytes ext=%s bytes=%d', ext, bytes.length);
          const response = await callCoreRpc<TranscribeResult>({
            method: 'openhuman.voice_transcribe_bytes',
            params: {
              audio_bytes: bytes,
              extension: ext,
              skip_cleanup: false,
            },
          });
          if (sessionIdRef.current !== sessionId) {
            console.debug('[dictation] ignoring stale transcription response session=%d', sessionId);
            return;
          }

          const text = response.text.trim();
          console.debug('[dictation] transcription result: %s', text || '(empty)');
          if (text) {
            dispatch(setTranscript(text));
          } else {
            dispatch(setError('No speech detected'));
          }
        } catch (err) {
          if (sessionIdRef.current !== sessionId) {
            console.debug('[dictation] ignoring stale transcription error session=%d', sessionId);
            return;
          }
          const msg = err instanceof Error ? err.message : 'Transcription failed';
          console.error('[dictation] transcription error:', msg, err);
          dispatch(setError(msg));
        }
      };

      console.debug('[dictation] starting MediaRecorder mimeType=%s', mimeType);
      recorder.start(100); // collect data every 100 ms
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Microphone access denied';
      console.error('[dictation] getUserMedia error:', msg, err);
      dispatch(setError(msg));
    }
  }, [status, dispatch]);

  const stopRecording = useCallback(() => {
    if (mediaRecorderRef.current && mediaRecorderRef.current.state !== 'inactive') {
      console.debug('[dictation] stopping MediaRecorder');
      mediaRecorderRef.current.stop();
      mediaRecorderRef.current = null;
    }
  }, []);

  const toggle = useCallback(() => {
    console.debug('[dictation] toggle called status=%s', status);
    if (status === 'recording') {
      stopRecording();
    } else if (status === 'idle' || status === 'ready' || status === 'error') {
      void startRecording();
    }
  }, [status, startRecording, stopRecording]);

  const dismiss = useCallback(() => {
    sessionIdRef.current += 1;
    stopRecording();
    dispatch(resetDictation());
  }, [stopRecording, dispatch]);

  useEffect(() => {
    toggleRef.current = toggle;
  }, [toggle]);

  // Re-register persisted hotkey on startup / change.
  useEffect(() => {
    void registerDictationHotkey(hotkey).catch(err => {
      console.warn('[dictation] auto register hotkey failed:', err);
      setTimeout(() => {
        void registerDictationHotkey(hotkey).catch(retryErr => {
          console.warn('[dictation] hotkey retry failed:', retryErr);
        });
      }, 2000);
    });
  }, [hotkey]);

  // Listen for global hotkey event from Tauri
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    listen<void>('dictation://toggle', () => {
      console.debug('[dictation] received dictation://toggle event');
      toggleRef.current();
    })
      .then(fn => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(err => {
        console.warn('[dictation] failed to listen for toggle event:', err);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // In-app fallback hotkey handler when global registration is unavailable.
  useEffect(() => {
    const parsed = parseHotkey(hotkey);
    if (!parsed) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.repeat) return;
      if (!matchesHotkeyEvent(event, parsed)) return;
      // Ignore focused editable fields to avoid hijacking typing shortcuts.
      const active = document.activeElement as HTMLElement | null;
      const isEditable =
        active instanceof HTMLInputElement ||
        active instanceof HTMLTextAreaElement ||
        !!active?.isContentEditable;
      if (isEditable) return;

      console.debug('[dictation] in-app hotkey matched, toggling');
      event.preventDefault();
      toggleRef.current();
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [hotkey]);

  return {
    status,
    isSupported,
    startRecording,
    stopRecording,
    toggle,
    dismiss,
  };
}
