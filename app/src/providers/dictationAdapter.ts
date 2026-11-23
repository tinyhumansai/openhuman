import type { DictationAdapter } from '@assistant-ui/react';
import debugFactory from 'debug';

import { isVoiceNotCompiledError, transcribeWithFactory } from '../features/human/voice/sttClient';
import { encodeBlobToWav } from '../features/human/voice/wavEncoder';

const log = debugFactory('openhuman:composer-dictation');

export const MAX_DICTATION_RECORDING_MS = 60_000;
export const DICTATION_START_TIMEOUT_MS = 30_000;
export const DICTATION_STOP_TIMEOUT_MS = 5_000;
export const DICTATION_TRANSCRIPTION_TIMEOUT_MS = 90_000;

export type DictationErrorCode =
  | 'microphone-unavailable'
  | 'permission-denied'
  | 'device-unavailable'
  | 'device-in-use'
  | 'recorder-failed'
  | 'no-audio'
  | 'no-speech'
  | 'transcription-failed'
  | 'voice-unavailable'
  | 'timed-out';

export type DictationSnapshot = {
  phase: 'idle' | 'starting' | 'recording' | 'transcribing';
  /** Content-free code; the surface translates it rather than exposing RPC errors. */
  error: DictationErrorCode | null;
};

type ActiveSession = { cancel: () => void };
type Status = DictationAdapter.Status;
type Result = DictationAdapter.Result;

const PREFERRED_MIMES = ['audio/mp4;codecs=mp4a.40.2', 'audio/mp4', 'audio/webm;codecs=opus'];

/** Presence checks only: asking for microphone permission remains a user action. */
export function isDictationCaptureSupported(): boolean {
  return (
    typeof MediaRecorder !== 'undefined' &&
    typeof navigator !== 'undefined' &&
    typeof navigator.mediaDevices?.getUserMedia === 'function'
  );
}

/** Prefer supported compressed audio; an empty value lets the browser choose. */
function pickRecorderMime(): string {
  if (typeof MediaRecorder.isTypeSupported !== 'function') return '';
  return PREFERRED_MIMES.find(mime => MediaRecorder.isTypeSupported(mime)) ?? '';
}

/** Map capture failures to localizable codes without retaining device details. */
function permissionErrorCode(error: unknown): DictationErrorCode {
  const name = typeof error === 'object' && error !== null && 'name' in error ? error.name : '';
  if (name === 'NotAllowedError' || name === 'SecurityError') return 'permission-denied';
  if (name === 'NotFoundError' || name === 'OverconstrainedError') return 'device-unavailable';
  if (name === 'NotReadableError') return 'device-in-use';
  return 'microphone-unavailable';
}

/** Keep one subscriber's exception from holding microphone resources open. */
function notify<T>(listeners: Set<(value: T) => void>, value: T): void {
  for (const listener of [...listeners]) {
    try {
      listener(value);
    } catch {
      log('subscriber threw');
    }
  }
}

/**
 * One runtime's editable composer dictation, using the shipped capture + STT path.
 * Stop finalizes the clip and emits one final result before its promise resolves.
 * Cancel discards it immediately; permission grants and STT replies arriving later
 * can only release resources. No transcript is sent as a chat message here.
 */
export class OpenHumanDictationAdapter implements DictationAdapter {
  // assistant-ui rebases dictation on setText, preserving edits made during capture.
  readonly disableInputDuringDictation = false;
  private snapshot: DictationSnapshot = { phase: 'idle', error: null };
  private readonly subscribers = new Set<() => void>();
  private active: ActiveSession | null = null;
  private nextSessionId = 0;

  /** Stable external-store snapshot, replaced only when the session changes. */
  getSnapshot = (): DictationSnapshot => this.snapshot;

  /** Subscribe to capture state and return an idempotent unsubscription function. */
  subscribe = (listener: () => void): (() => void) => {
    this.subscribers.add(listener);
    return () => this.subscribers.delete(listener);
  };

  /** Notify the host of a content-free phase or error transition. */
  private publish(snapshot: DictationSnapshot): void {
    this.snapshot = snapshot;
    notify(this.subscribers, undefined);
  }

  /** Discard the current clip and invalidate any pending transcription result. */
  cancel = (): void => {
    this.active?.cancel();
  };

  /** Reusable after cleanup, including React StrictMode's effect re-setup. */
  dispose = (): void => {
    this.cancel();
    this.subscribers.clear();
  };

  /** Replace any previous session and defer capture until listeners can attach. */
  listen = (): DictationAdapter.Session => {
    this.cancel();
    const sessionId = ++this.nextSessionId;
    let status: Status = { type: 'starting' };
    let ended = false;
    let stopping = false;
    let finalizing = false;
    let stream: MediaStream | null = null;
    let recorder: MediaRecorder | null = null;
    let chunks: Blob[] = [];
    let detachTracks: (() => void) | null = null;
    let startupTimer: ReturnType<typeof setTimeout> | undefined;
    let recordingTimer: ReturnType<typeof setTimeout> | undefined;
    let stopTimer: ReturnType<typeof setTimeout> | undefined;
    let transcriptionTimer: ReturnType<typeof setTimeout> | undefined;
    const speechListeners = new Set<(result: Result) => void>();
    const startListeners = new Set<() => void>();
    const endListeners = new Set<(result: Result) => void>();
    let resolveCompletion!: () => void;
    const completion = new Promise<void>(resolve => {
      resolveCompletion = resolve;
    });

    /** Detach device-loss handlers before stopping every owned microphone track. */
    const releaseStream = () => {
      detachTracks?.();
      detachTracks = null;
      if (stream) {
        for (const track of stream.getTracks()) {
          try {
            track.stop();
          } catch {
            log('session=%d track cleanup failed', sessionId);
          }
        }
        stream = null;
      }
    };

    /** Terminate exactly once, release resources, and settle every stop caller. */
    const finish = (
      reason: 'stopped' | 'cancelled' | 'error',
      error: DictationErrorCode | null = null,
      transcript = ''
    ) => {
      if (ended) return;
      ended = true;
      status = { type: 'ended', reason };
      clearTimeout(startupTimer);
      clearTimeout(recordingTimer);
      clearTimeout(stopTimer);
      clearTimeout(transcriptionTimer);
      if (recorder) {
        recorder.ondataavailable = null;
        recorder.onstop = null;
        recorder.onerror = null;
        try {
          if (recorder.state !== 'inactive') recorder.stop();
        } catch {
          log('session=%d recorder cleanup failed', sessionId);
        }
        recorder = null;
      }
      releaseStream();
      chunks = [];
      if (this.active === active) {
        this.active = null;
        this.publish({ phase: 'idle', error });
      }
      log('session=%d ended reason=%s error=%s', sessionId, reason, error ?? 'none');
      // assistant-ui uses this event to clean its session synchronously. Merely
      // changing status would leave its controls active until the next 100ms poll.
      notify(endListeners, { transcript, isFinal: true });
      speechListeners.clear();
      startListeners.clear();
      endListeners.clear();
      resolveCompletion();
    };

    const active: ActiveSession = { cancel: () => finish('cancelled') };
    this.active = active;
    this.publish({ phase: 'starting', error: null });

    /** Transcribe the completed clip, retry as WAV, and emit only a live result. */
    const finalize = async () => {
      if (ended || finalizing) return;
      finalizing = true;
      stopping = true;
      clearTimeout(recordingTimer);
      clearTimeout(stopTimer);
      releaseStream();
      this.publish({ phase: 'transcribing', error: null });
      if (ended) return;
      const blob = new Blob(chunks, {
        type: recorder?.mimeType || chunks[0]?.type || 'audio/webm',
      });
      chunks = [];
      if (blob.size === 0) {
        finish('error', 'no-audio');
        return;
      }
      transcriptionTimer = setTimeout(
        () => finish('error', 'timed-out'),
        DICTATION_TRANSCRIPTION_TIMEOUT_MS
      );
      try {
        let text: string;
        try {
          text = await transcribeWithFactory(blob);
        } catch (error) {
          if (ended) return;
          if (isVoiceNotCompiledError(error)) throw error;
          // Reuse the shipped voice path's portable PCM fallback. The existing
          // timer bounds native STT, conversion, and this one retry together.
          log('session=%d native transcription failed; retrying as WAV', sessionId);
          const wav = await encodeBlobToWav(blob);
          if (ended) return;
          text = await transcribeWithFactory(wav);
        }
        const transcript = text.trim();
        if (ended) return;
        if (!transcript) {
          finish('error', 'no-speech');
          return;
        }
        // Emit while assistant-ui is still subscribed, before terminal status or
        // speechEnd can clear its session. Its composer owns appending to drafts.
        notify(speechListeners, { transcript, isFinal: true });
        finish('stopped', null, transcript);
      } catch (error) {
        if (ended) return;
        finish(
          'error',
          isVoiceNotCompiledError(error) ? 'voice-unavailable' : 'transcription-failed'
        );
      }
    };

    /** Finish the clip once; repeated calls share the same bounded completion. */
    const stop = (): Promise<void> => {
      if (ended || stopping) return completion;
      if (status.type === 'starting') {
        // getUserMedia cannot be aborted. Invalidate it now and release any late
        // grant instead of recording after the user has pressed Stop.
        finish('cancelled');
        return completion;
      }
      stopping = true;
      clearTimeout(recordingTimer);
      this.publish({ phase: 'transcribing', error: null });
      if (ended) return completion;
      stopTimer = setTimeout(() => finish('error', 'timed-out'), DICTATION_STOP_TIMEOUT_MS);
      try {
        recorder?.stop();
        // A broken recorder must not keep the OS microphone indicator on while
        // we wait for its final event (or for the stop bound to expire).
        releaseStream();
      } catch {
        finish('error', 'recorder-failed');
      }
      return completion;
    };

    /** Request capture and release late permission grants after cancellation. */
    const start = async () => {
      if (ended) return;
      if (!isDictationCaptureSupported()) {
        finish('error', 'microphone-unavailable');
        return;
      }
      let granted: MediaStream;
      try {
        granted = await navigator.mediaDevices.getUserMedia({
          audio: {
            channelCount: 1,
            sampleRate: 48000,
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
          },
        });
      } catch (error) {
        if (!ended) finish('error', permissionErrorCode(error));
        return;
      }
      if (ended) {
        for (const track of granted.getTracks()) {
          try {
            track.stop();
          } catch {
            log('session=%d late track cleanup failed', sessionId);
          }
        }
        return;
      }
      clearTimeout(startupTimer);
      stream = granted;
      const tracks = granted.getTracks();
      const deviceLost = () => finish('error', 'device-unavailable');
      for (const track of tracks) track.addEventListener('ended', deviceLost);
      detachTracks = () => {
        for (const track of tracks) track.removeEventListener('ended', deviceLost);
      };
      if (tracks.some(track => track.readyState === 'ended')) {
        deviceLost();
        return;
      }
      try {
        const mime = pickRecorderMime();
        recorder = new MediaRecorder(granted, {
          audioBitsPerSecond: 128_000,
          ...(mime ? { mimeType: mime } : {}),
        });
        recorder.ondataavailable = event => {
          if (!ended && event.data?.size > 0) chunks.push(event.data);
        };
        recorder.onstop = () => {
          void finalize();
        };
        recorder.onerror = () => finish('error', 'recorder-failed');
        recorder.start();
        if (ended) return;
        status = { type: 'running' };
        this.publish({ phase: 'recording', error: null });
        if (ended) return;
        // Only Finish authorizes transcription; a duration limit must never
        // upload a clip the user may still intend to discard.
        recordingTimer = setTimeout(() => finish('error', 'timed-out'), MAX_DICTATION_RECORDING_MS);
        log('session=%d recording started', sessionId);
        notify(startListeners, undefined);
      } catch {
        finish('error', 'recorder-failed');
      }
    };

    if (!ended) {
      startupTimer = setTimeout(() => finish('error', 'timed-out'), DICTATION_START_TIMEOUT_MS);
    }
    // listen() is synchronous; allow the composer to install all three event
    // subscriptions before an unsupported browser or synchronous denial ends it.
    void Promise.resolve().then(start);

    return {
      /** Expose the assistant-ui session status independently of host UI phases. */
      get status() {
        return status;
      },
      stop,
      cancel: active.cancel,
      /** Receive the final transcript while the composer session is still active. */
      onSpeech(callback) {
        speechListeners.add(callback);
        return () => speechListeners.delete(callback);
      },
      /** Receive confirmation that microphone recording actually started. */
      onSpeechStart(callback) {
        startListeners.add(callback);
        return () => startListeners.delete(callback);
      },
      /** Receive terminal cleanup, including cancellation with an empty result. */
      onSpeechEnd(callback) {
        endListeners.add(callback);
        return () => endListeners.delete(callback);
      },
    };
  };
}

/** Create an independent capture lifecycle for one chat runtime. */
export function createOpenHumanDictationAdapter(): OpenHumanDictationAdapter {
  return new OpenHumanDictationAdapter();
}
