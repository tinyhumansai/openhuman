/**
 * VoiceNotePlayer - play/pause, scrub, duration and transcript disclosure for
 * one voice note, user recording or assistant spoken reply alike.
 *
 * Audio comes from either `src` (a draft's object URL) or `noteId`, fetched
 * from the core on first play so a thread with many notes loads none of them
 * until asked.
 */
import { PauseIcon, PlayIcon } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState } from 'react';

import { trackAnalyticsEvent } from '../../components/analytics';
import Button from '../../components/ui/Button';
import {
  CollapsibleContent,
  CollapsibleRoot,
  CollapsibleTrigger,
} from '../../components/ui/Collapsible';
import Slider from '../../components/ui/Slider';
import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { proceduralVisemes } from '../human/voice/ttsClient';
import { formatDuration, readVoiceNote } from './voiceNoteApi';

export interface VoiceNotePlayerProps {
  noteId?: string;
  /** Ready-to-play URL (a draft preview); skips the core fetch. */
  src?: string;
  durationMs?: number;
  waveform?: number[];
  /** When set, a transcript disclosure is shown under the player. */
  transcript?: string;
  /** Animate the shared mascot during assistant reply playback. */
  mascotPlayback?: boolean;
  visemes?: Array<{ viseme: string; start_ms: number; end_ms: number }>;
  speechText?: string;
  className?: string;
}

/** Keyed on the audio source so a different note starts from a clean player. */
export function VoiceNotePlayer(props: VoiceNotePlayerProps) {
  return <VoiceNotePlayerBody key={`${props.noteId ?? ''}|${props.src ?? ''}`} {...props} />;
}

function VoiceNotePlayerBody({
  noteId,
  src,
  durationMs,
  waveform,
  transcript,
  mascotPlayback = false,
  visemes,
  speechText = '',
  className,
}: VoiceNotePlayerProps) {
  const { t } = useT();
  const playbackOwner = useId();
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const objectUrlRef = useRef<string | null>(null);
  const playbackRafRef = useRef<number | null>(null);
  const [playing, setPlaying] = useState(false);
  const [positionMs, setPositionMs] = useState(0);
  const [loadedMs, setLoadedMs] = useState<number | null>(null);
  const totalMs = loadedMs ?? durationMs ?? 0;
  const frames = visemes?.length ? visemes : proceduralVisemes(speechText, totalMs);
  const framesRef = useRef(frames);
  framesRef.current = frames;

  const publishMascot = useCallback(
    (active: boolean, atMs = 0, claim = false) => {
      if (!mascotPlayback) return;
      let viseme = 'sil';
      if (active) {
        const frame = framesRef.current.find(item => atMs >= item.start_ms && atMs < item.end_ms);
        viseme = frame?.viseme ?? 'sil';
      }
      window.dispatchEvent(
        new CustomEvent('voice-note-mascot', {
          detail: { owner: playbackOwner, active, viseme, claim },
        })
      );
    },
    [mascotPlayback, playbackOwner]
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const release = useCallback(() => {
    if (playbackRafRef.current !== null) {
      window.cancelAnimationFrame(playbackRafRef.current);
      playbackRafRef.current = null;
    }
    audioRef.current?.pause();
    audioRef.current = null;
    publishMascot(false);
    if (objectUrlRef.current) {
      URL.revokeObjectURL(objectUrlRef.current);
      objectUrlRef.current = null;
    }
  }, [publishMascot]);

  // Release the element and any object URL when the player goes away.
  useEffect(() => release, [release]);

  const ensureAudio = useCallback(async (): Promise<HTMLAudioElement | null> => {
    if (audioRef.current) return audioRef.current;
    let url = src;
    if (!url) {
      if (!noteId) return null;
      setLoading(true);
      try {
        const blob = await readVoiceNote(noteId);
        if (!blob) throw new Error('voice note audio missing');
        url = URL.createObjectURL(blob);
        objectUrlRef.current = url;
      } catch (err) {
        console.warn('[voice-note] load failed', err);
        setError(true);
        return null;
      } finally {
        setLoading(false);
      }
    }
    const audio = new window.Audio(url);
    const publishAtCurrentTime = (claim = false) => {
      const atMs = audio.currentTime * 1000;
      setPositionMs(atMs);
      publishMascot(!audio.paused && !audio.ended, atMs, claim);
    };
    const stopSampling = () => {
      if (playbackRafRef.current !== null) {
        window.cancelAnimationFrame(playbackRafRef.current);
        playbackRafRef.current = null;
      }
    };
    const sampleWhilePlaying = () => {
      if (audio.paused || audio.ended) {
        playbackRafRef.current = null;
        return;
      }
      publishAtCurrentTime();
      playbackRafRef.current = window.requestAnimationFrame(sampleWhilePlaying);
    };
    audio.addEventListener('timeupdate', () => publishAtCurrentTime());
    audio.addEventListener('loadedmetadata', () => {
      if (Number.isFinite(audio.duration) && audio.duration > 0) {
        setLoadedMs(audio.duration * 1000);
      }
    });
    audio.addEventListener('ended', () => {
      stopSampling();
      setPlaying(false);
      setPositionMs(0);
      publishMascot(false);
    });
    audio.addEventListener('pause', () => {
      stopSampling();
      setPlaying(false);
      publishMascot(false);
    });
    audio.addEventListener('play', () => {
      setPlaying(true);
      publishAtCurrentTime(true);
      stopSampling();
      playbackRafRef.current = window.requestAnimationFrame(sampleWhilePlaying);
    });
    audioRef.current = audio;
    return audio;
  }, [noteId, src, publishMascot]);

  const toggle = useCallback(async () => {
    const audio = await ensureAudio();
    if (!audio) return;
    if (audio.paused) {
      trackAnalyticsEvent('voice_note_played', {});
      await audio.play().catch(err => {
        console.warn('[voice-note] play failed', err);
        setError(true);
      });
    } else {
      audio.pause();
    }
  }, [ensureAudio]);

  const seek = useCallback(
    async ([value]: number[]) => {
      const audio = await ensureAudio();
      if (!audio) return;
      audio.currentTime = value / 1000;
      setPositionMs(value);
      if (!audio.paused) publishMascot(true, value);
    },
    [ensureAudio, publishMascot]
  );

  const progress = totalMs > 0 ? Math.min(1, positionMs / totalMs) : 0;
  const label = t('voiceNote.label').replace('{duration}', formatDuration(totalMs));

  return (
    <div
      data-testid="voice-note-player"
      className={cn('flex min-w-56 flex-col gap-1.5', className)}>
      <div className="flex items-center gap-2">
        <Button
          type="button"
          iconOnly
          variant="secondary"
          size="sm"
          analyticsId="voice-note-toggle"
          data-testid="voice-note-toggle"
          aria-label={playing ? t('voiceNote.pause') : t('voiceNote.play')}
          disabled={loading}
          onClick={() => void toggle()}>
          {playing ? <PauseIcon className="size-4" /> : <PlayIcon className="size-4" />}
        </Button>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          {waveform && waveform.length > 0 ? (
            <div
              data-testid="voice-note-waveform"
              aria-hidden="true"
              className="flex h-6 items-center gap-px">
              {waveform.map((bar, index) => (
                <span
                  key={index}
                  className={cn(
                    'w-0.5 flex-1 rounded-full',
                    index / waveform.length < progress ? 'bg-primary-500' : 'bg-surface-strong'
                  )}
                  style={{ height: `${Math.max(12, Math.round(bar * 100))}%` }}
                />
              ))}
            </div>
          ) : null}
          <Slider
            size="sm"
            min={0}
            max={Math.max(totalMs, 1)}
            step={50}
            value={[Math.min(positionMs, Math.max(totalMs, 1))]}
            disabled={totalMs <= 0}
            thumbLabels={[t('voiceNote.seek')]}
            onValueChange={values => void seek(values)}
          />
        </div>
        <span
          data-testid="voice-note-time"
          aria-label={label}
          className="text-content-muted w-10 shrink-0 text-right text-xs tabular-nums">
          {formatDuration(playing || positionMs > 0 ? positionMs : totalMs)}
        </span>
      </div>
      {error ? (
        <p role="alert" className="text-coral-600 text-xs">
          {t('voiceNote.loadFailed')}
        </p>
      ) : null}
      {transcript !== undefined ? (
        <CollapsibleRoot>
          <CollapsibleTrigger data-testid="voice-note-transcript-toggle">
            {t('voiceNote.transcript')}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <p data-testid="voice-note-transcript" className="text-sm whitespace-pre-wrap">
              {transcript.trim() || t('voiceNote.noTranscript')}
            </p>
          </CollapsibleContent>
        </CollapsibleRoot>
      ) : null}
    </div>
  );
}

export default VoiceNotePlayer;
