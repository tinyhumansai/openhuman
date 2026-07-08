import { useEffect, useRef, useState } from 'react';

import { synthesizeSpeech } from '../../../features/human/voice/ttsClient';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  selectEffectiveMascotVoiceId,
  selectMascotVoiceFor,
  selectMascotVoiceGender,
  setMascotVoice,
} from '../../../store/mascotSlice';
import Button from '../../ui/Button';
import { SettingsSelect, SettingsTextField } from '../controls';
import { ELEVENLABS_VOICE_PRESETS, isCuratedVoicePreset } from './elevenlabsVoicePresets';

interface PerMascotVoiceRowProps {
  /** Manifest mascot id this row controls the voice for. Writes land in
   *  `mascotVoices[mascotId]` via `setMascotVoice`. */
  mascotId: string;
  /** Human-readable heading for the row (e.g. the mascot's name), already
   *  localized by the caller. */
  label: string;
  /** Stable test hook so both the primary and secondary rows expose
   *  distinct `data-testid`s (`mascot-voice-{primary,secondary}-*`). */
  testIdPrefix: string;
}

/**
 * Per-mascot reply-voice control (issue #4277). A trimmed sibling of the
 * primary voice section in `MascotPanel`: it reuses the same preset
 * dropdown + custom-paste + guarded `synthesizeSpeech` preview, but writes
 * to `mascotVoices[mascotId]` via `setMascotVoice` instead of the single
 * `voiceId`. The current value is the per-mascot override
 * (`selectMascotVoiceFor`) falling back to the effective single voice
 * (`selectEffectiveMascotVoiceId`), so a mascot with no override sounds
 * exactly like the single-voice behaviour until the user picks one.
 *
 * Extracted from `MascotPanel` to keep that file within the ~500-line
 * budget while the per-mascot map (primary + secondary) doubles the voice
 * UI. Each instance owns its own preview-abort guard (`previewRequestIdRef`)
 * so the two rows never share an in-flight preview.
 */
const PerMascotVoiceRow = ({ mascotId, label, testIdPrefix }: PerMascotVoiceRowProps) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  // Per-mascot override, or the effective single voice when unset — the
  // same resolution the meeting join path uses, so the picker shows what
  // the mascot will actually speak with.
  const overrideVoiceId = useAppSelector(selectMascotVoiceFor(mascotId));
  const effectiveVoiceId = useAppSelector(selectEffectiveMascotVoiceId);
  const currentVoiceId = overrideVoiceId ?? effectiveVoiceId;
  // Reuse the global gender bucket to filter the preset dropdown — the
  // per-mascot control only overrides the voice id, not the gender filter.
  const voiceGender = useAppSelector(selectMascotVoiceGender);

  // Paste-mode is sticky for the same reason as the primary control: a
  // curated preset id and a mid-paste custom id both leave the stored
  // value looking like a known id, so we can't derive the mode from it.
  const [voiceDraft, setVoiceDraft] = useState<string>(overrideVoiceId ?? '');
  const [voicePasteMode, setVoicePasteMode] = useState<boolean>(false);
  const [isPreviewingVoice, setIsPreviewingVoice] = useState(false);
  const [voicePreviewError, setVoicePreviewError] = useState<string | null>(null);
  const previewAudioRef = useRef<HTMLAudioElement | null>(null);
  // Monotonically-bumped preview-request id, mirroring the primary
  // control: unmount + each new preview both increment it so an in-flight
  // `synthesizeSpeech(...)` whose resolve loses the race bails before it
  // touches refs / state.
  const previewRequestIdRef = useRef(0);

  // Stop any in-flight preview audio on unmount and invalidate a pending
  // `synthesizeSpeech(...)` so a late resolve can't start audio for a row
  // the user has already navigated away from.
  useEffect(() => {
    return () => {
      previewRequestIdRef.current += 1;
      if (previewAudioRef.current) {
        previewAudioRef.current.pause();
        previewAudioRef.current.src = '';
        previewAudioRef.current = null;
      }
    };
  }, []);

  // Presets the dropdown should expose: always include the current voice
  // (so the controlled select never points at an absent option) plus the
  // active gender bucket and any '*' fallback voices.
  const visiblePresets = ELEVENLABS_VOICE_PRESETS.filter(
    p => p.id === currentVoiceId || p.gender === voiceGender || p.locales.includes('*')
  );

  // A custom (non-curated) override keeps the paste editor open so the
  // stored id stays visible; the effective-voice fallback is never treated
  // as "custom" because the mascot has no explicit override yet.
  const isCustomVoice =
    voicePasteMode || (overrideVoiceId != null && !isCuratedVoicePreset(overrideVoiceId));

  const onPresetChange = (next: string) => {
    if (next === '__custom__') {
      setVoicePasteMode(true);
      setVoiceDraft(overrideVoiceId ?? '');
      return;
    }
    setVoicePasteMode(false);
    setVoicePreviewError(null);
    setVoiceDraft(next);
    dispatch(setMascotVoice({ mascotId, voiceId: next }));
  };

  const onSavePaste = () => {
    setVoicePreviewError(null);
    const trimmed = voiceDraft.trim();
    setVoiceDraft(trimmed);
    dispatch(setMascotVoice({ mascotId, voiceId: trimmed.length > 0 ? trimmed : null }));
  };

  const onVoiceReset = () => {
    setVoicePreviewError(null);
    setVoicePasteMode(false);
    setVoiceDraft('');
    dispatch(setMascotVoice({ mascotId, voiceId: null }));
  };

  const onVoicePreview = async () => {
    // Same abort guard as the primary control: reserve a fresh id, and let
    // a stale resolve detect that a newer preview (or unmount) superseded
    // it before it mutates state or plays audio.
    const requestId = ++previewRequestIdRef.current;
    setIsPreviewingVoice(true);
    setVoicePreviewError(null);
    if (previewAudioRef.current) {
      previewAudioRef.current.pause();
      previewAudioRef.current.src = '';
      previewAudioRef.current = null;
    }
    try {
      const tts = await synthesizeSpeech(t('settings.mascot.voice.previewText'), {
        voiceId: currentVoiceId,
      });
      if (previewRequestIdRef.current !== requestId) return;
      const src = `data:${tts.audio_mime || 'audio/mpeg'};base64,${tts.audio_base64}`;
      const audio = new window.Audio(src);
      previewAudioRef.current = audio;
      await audio.play();
    } catch (err) {
      if (previewRequestIdRef.current !== requestId) return;
      const message = err instanceof Error ? err.message : t('settings.mascot.voice.previewError');
      setVoicePreviewError(message);
    } finally {
      if (previewRequestIdRef.current === requestId) setIsPreviewingVoice(false);
    }
  };

  return (
    <div className="bg-surface rounded-xl border border-line p-4 space-y-3">
      <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
        {label}
      </span>

      {/* Preset dropdown — mirrors the primary control's label + select combo */}
      <label className="block space-y-1">
        <span className="sr-only">{t('settings.mascot.voice.presetHeading')}</span>
        <SettingsSelect
          aria-label={`${label} — ${t('settings.mascot.voice.presetHeading')}`}
          data-testid={`${testIdPrefix}-select`}
          value={isCustomVoice ? '__custom__' : currentVoiceId}
          onChange={e => onPresetChange(e.target.value)}
          className="w-full">
          {visiblePresets.map(v => (
            <option key={v.id} value={v.id}>
              {v.label}
            </option>
          ))}
          <option value="__custom__">{t('settings.mascot.voice.customOption')}</option>
        </SettingsSelect>
      </label>

      {isCustomVoice && (
        <label className="block space-y-1">
          <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
            {t('settings.mascot.voice.customHeading')}
          </span>
          <div className="flex gap-2">
            <SettingsTextField
              aria-label={`${label} — ${t('settings.mascot.voice.customHeading')}`}
              data-testid={`${testIdPrefix}-input`}
              value={voiceDraft}
              placeholder={t('settings.mascot.voice.customPlaceholder')}
              onChange={e => setVoiceDraft(e.target.value)}
              className="flex-1"
            />
            <Button
              type="button"
              variant="primary"
              size="xs"
              data-testid={`${testIdPrefix}-save-paste`}
              onClick={onSavePaste}
              disabled={voiceDraft.trim() === (overrideVoiceId ?? '').trim()}>
              {t('common.save')}
            </Button>
          </div>
        </label>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="primary"
          size="xs"
          data-testid={`${testIdPrefix}-preview`}
          onClick={() => void onVoicePreview()}
          disabled={isPreviewingVoice}
          className="bg-emerald-600 hover:bg-emerald-700 dark:bg-emerald-600 dark:hover:bg-emerald-500">
          {isPreviewingVoice
            ? t('settings.mascot.voice.previewing')
            : t('settings.mascot.voice.preview')}
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="xs"
          data-testid={`${testIdPrefix}-reset`}
          onClick={onVoiceReset}
          disabled={overrideVoiceId == null}>
          {t('settings.mascot.voice.reset')}
        </Button>
        <span
          data-testid={`${testIdPrefix}-current`}
          className="ml-1 text-[11px] text-content-muted truncate max-w-[18rem]"
          title={currentVoiceId}>
          {t('settings.mascot.voice.current')}: <code className="font-mono">{currentVoiceId}</code>
        </span>
      </div>

      {voicePreviewError && (
        <div
          data-testid={`${testIdPrefix}-preview-error`}
          className="rounded-md border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">
          {t('settings.mascot.voice.previewError')}: {voicePreviewError}
        </div>
      )}
    </div>
  );
};

export default PerMascotVoiceRow;
