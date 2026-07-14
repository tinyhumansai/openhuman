import { useEffect, useMemo, useRef, useState } from 'react';

import { CustomGifMascot, ManifestRiveMascot, RiveMascot } from '../../../features/human/Mascot';
import { useMascotManifest } from '../../../features/human/Mascot/manifest/useMascotManifest';
import {
  getMascotPalette,
  hexToArgbInt,
  type MascotColor,
} from '../../../features/human/Mascot/mascotPalette';
import { synthesizeSpeech } from '../../../features/human/voice/ttsClient';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  DEFAULT_MASCOT_COLOR,
  isCustomMascotGifUrl,
  type MascotVoiceGender,
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectEffectiveMascotVoiceId,
  selectMascotColor,
  selectMascotVoiceGender,
  selectMascotVoiceId,
  selectMascotVoiceUseLocaleDefault,
  selectSecondaryMascotId,
  selectSelectedMascotId,
  setCustomMascotGifUrl,
  setCustomPrimaryColor,
  setCustomSecondaryColor,
  setMascotColor,
  setMascotVoiceGender,
  setMascotVoiceId,
  setMascotVoiceUseLocaleDefault,
  setSecondaryMascotId,
  setSelectedMascotId,
  SUPPORTED_MASCOT_COLORS,
} from '../../../store/mascotSlice';
import Button from '../../ui/Button';
import { SettingsSelect, SettingsTextField } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import {
  defaultVoiceIdForLocale,
  ELEVENLABS_VOICE_PRESETS,
  isCuratedVoicePreset,
} from './elevenlabsVoicePresets';
import PerMascotVoiceRow from './PerMascotVoiceRow';

interface ColorOption {
  id: MascotColor;
  /** i18n key for the swatch label; resolved at render time so the locale can
   *  change without re-creating the array. */
  labelKey: string;
}

const COLOR_OPTIONS: ColorOption[] = [
  { id: 'yellow', labelKey: 'settings.mascot.colorYellow' },
  { id: 'burgundy', labelKey: 'settings.mascot.colorBurgundy' },
  { id: 'black', labelKey: 'settings.mascot.colorBlack' },
  { id: 'navy', labelKey: 'settings.mascot.colorNavy' },
  { id: 'custom', labelKey: 'settings.mascot.colorCustom' },
];

interface MascotPanelProps {
  /** When true the panel is hosted inside another settings page (the
   *  Personality & Face tabs) — skip the standalone SettingsHeader chrome. */
  embedded?: boolean;
}

const MascotPanel = ({ embedded = false }: MascotPanelProps) => {
  const { t, locale } = useT();
  const dispatch = useAppDispatch();
  const storedColor = useAppSelector(selectMascotColor);
  const customPrimary = useAppSelector(selectCustomPrimaryColor);
  const customSecondary = useAppSelector(selectCustomSecondaryColor);
  const selectedMascotId = useAppSelector(selectSelectedMascotId);
  const secondaryMascotId = useAppSelector(selectSecondaryMascotId);
  const customMascotGifUrl = useAppSelector(selectCustomMascotGifUrl);
  const storedVoiceId = useAppSelector(selectMascotVoiceId);
  const voiceGender = useAppSelector(selectMascotVoiceGender);
  const useLocaleDefault = useAppSelector(selectMascotVoiceUseLocaleDefault);
  const effectiveVoiceId = useAppSelector(selectEffectiveMascotVoiceId);

  // Mascot library, sourced from the published GitHub manifest
  // (tinyhumansai/mascots). `entry` is the resolved active mascot (selection
  // or default); `manifest.mascots` drives the picker list. Each entry carries
  // its full stateEngine inline, so there is no per-id detail round trip.
  const {
    manifest,
    entry: activeEntry,
    loading: manifestLoading,
    error: manifestError,
  } = useMascotManifest();
  const [customGifDraft, setCustomGifDraft] = useState<string>(customMascotGifUrl ?? '');
  const [customGifError, setCustomGifError] = useState<string | null>(null);

  // Voice picker state — paste-mode is sticky because we can't derive it
  // from the stored value alone (a curated preset id and "user is
  // mid-paste" both leave `storedVoiceId` looking like a known id).
  const [voiceDraft, setVoiceDraft] = useState<string>(storedVoiceId ?? '');
  const [voicePasteMode, setVoicePasteMode] = useState<boolean>(false);
  const [isPreviewingVoice, setIsPreviewingVoice] = useState(false);
  const [voicePreviewError, setVoicePreviewError] = useState<string | null>(null);
  const previewAudioRef = useRef<HTMLAudioElement | null>(null);
  // Monotonically-bumped preview-request id. Unmount + each new preview
  // both increment it so any in-flight `synthesizeSpeech(...)` whose
  // resolve loses the race is detected and bails out before touching
  // refs / state — covers the "user navigates away mid-fetch" case the
  // earlier audio-only cleanup missed.
  const previewRequestIdRef = useRef(0);

  // Stop any in-flight preview audio when the panel unmounts. Also
  // bump the preview request id so a `synthesizeSpeech(...)` that
  // resolves after unmount can detect the staleness and bail.
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

  const handleSelectMascot = (id: string | null) => {
    dispatch(setSelectedMascotId(id));
    setCustomGifError(null);
    setCustomGifDraft('');
    // Selecting a mascot id already clears the custom GIF in the reducer; the
    // null ("default") case has to clear it here so the stage falls back to
    // the default manifest mascot rather than the GIF.
    if (id == null) dispatch(setCustomMascotGifUrl(null));
    // A newly-picked primary that collides with the current secondary would
    // leave both slots pointing at the same mascot; clear the secondary so
    // the duo never duplicates. The reducer's `selectDualMascotEnabled`
    // guard already treats a collision as single-mascot, but clearing here
    // keeps the picker's rendered state honest.
    if (id != null && id === secondaryMascotId) dispatch(setSecondaryMascotId(null));
  };

  // ── Second-mascot picker (issue #4277) ───────────────────────────
  // Enable / clear the meeting duo's second mascot. `null` (the "None"
  // option) drops back to single-mascot. Picking the primary's id is
  // disabled in the dropdown, so this only ever dispatches a distinct id
  // or null.
  const handleSelectSecondaryMascot = (id: string | null) => {
    dispatch(setSecondaryMascotId(id));
  };

  const onSaveCustomGif = () => {
    const trimmed = customGifDraft.trim();
    setCustomGifDraft(trimmed);
    if (trimmed.length === 0) {
      setCustomGifError(null);
      dispatch(setCustomMascotGifUrl(null));
      return;
    }
    if (!isCustomMascotGifUrl(trimmed)) {
      setCustomGifError(t('settings.mascot.customGifError'));
      return;
    }
    setCustomGifError(null);
    dispatch(setCustomMascotGifUrl(trimmed));
  };

  const onResetCustomGif = () => {
    setCustomGifDraft('');
    setCustomGifError(null);
    dispatch(setCustomMascotGifUrl(null));
  };

  // Filter the menu to colors the asset pipeline currently supports — guards
  // against an older persisted value pointing at a variant a future build
  // removed. The selected swatch still highlights iff the stored color is
  // present; otherwise we silently fall back to the default for the preview.
  const available = COLOR_OPTIONS.filter(opt =>
    (SUPPORTED_MASCOT_COLORS as readonly string[]).includes(opt.id)
  );
  const activeColor: MascotColor = (SUPPORTED_MASCOT_COLORS as readonly string[]).includes(
    storedColor
  )
    ? storedColor
    : DEFAULT_MASCOT_COLOR;

  const handleSelect = (color: MascotColor) => {
    if (color === storedColor) return;
    dispatch(setMascotColor(color));
  };

  // ── Voice picker handlers ────────────────────────────────────────
  // Presets the dropdown should expose. Always include the default
  // mascot voice (regardless of its gender) so the user can fall back
  // without untoggling the gender filter first. Also always include
  // the currently-active preset id — otherwise flipping the gender
  // filter leaves the controlled `<select>` pointing at an id with
  // no matching `<option>`, and the picker stops reflecting the real
  // selection.
  const visiblePresets = ELEVENLABS_VOICE_PRESETS.filter(
    p => p.id === effectiveVoiceId || p.gender === voiceGender || p.locales.includes('*')
  );

  const onGenderChange = (next: MascotVoiceGender) => {
    dispatch(setMascotVoiceGender(next));
    const firstPreset = ELEVENLABS_VOICE_PRESETS.find(p => p.gender === next);
    if (firstPreset) {
      setVoicePasteMode(false);
      setVoicePreviewError(null);
      setVoiceDraft(firstPreset.id);
      dispatch(setMascotVoiceId(firstPreset.id));
    }
  };

  const onLocaleDefaultToggle = (next: boolean) => {
    dispatch(setMascotVoiceUseLocaleDefault(next));
  };

  // All slice writes flow through this component, so the local draft +
  // preview-error state can be reset inside the same handler that
  // dispatches `setMascotVoiceId(...)` — no `useEffect` mirror needed
  // (and the rule `react-hooks/set-state-in-effect` flags effect-based
  // mirrors as a smell).
  const onPresetChange = (next: string) => {
    if (next === '__custom__') {
      setVoicePasteMode(true);
      setVoiceDraft(storedVoiceId ?? '');
      return;
    }
    setVoicePasteMode(false);
    setVoicePreviewError(null);
    setVoiceDraft(next);
    dispatch(setMascotVoiceId(next));
  };

  const onSavePaste = () => {
    setVoicePreviewError(null);
    const trimmed = voiceDraft.trim();
    setVoiceDraft(trimmed);
    dispatch(setMascotVoiceId(trimmed.length > 0 ? trimmed : null));
  };

  const onVoiceReset = () => {
    setVoicePreviewError(null);
    setVoicePasteMode(false);
    setVoiceDraft('');
    dispatch(setMascotVoiceId(null));
  };

  const onVoicePreview = async () => {
    // Each click reserves a fresh request id; the unmount cleanup and
    // every subsequent click bump the ref, so a stale `synthesizeSpeech`
    // resolve can detect that the user has moved on before it mutates
    // state or starts audio for a preview that's no longer wanted.
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
        voiceId: effectiveVoiceId,
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

  const localeDefaultVoiceId = defaultVoiceIdForLocale(locale, voiceGender);
  const presetPickerDisabled = useLocaleDefault;
  const isCustomVoice =
    !presetPickerDisabled && (voicePasteMode || !isCuratedVoicePreset(effectiveVoiceId));

  const activePalette = getMascotPalette(activeColor);
  const primaryColorArgb = useMemo(
    () => hexToArgbInt(activeColor === 'custom' ? customPrimary : activePalette.bodyFill),
    [activeColor, customPrimary, activePalette]
  );
  const secondaryColorArgb = useMemo(
    () => hexToArgbInt(activeColor === 'custom' ? customSecondary : activePalette.neckShadowColor),
    [activeColor, customSecondary, activePalette]
  );

  const body = (
    <>
      {/* ── Mascot preview (intentional bespoke visual) ───────────── */}
      <div className="flex justify-center">
        <div style={{ width: 180, height: 180 }}>
          <RiveMascot
            face="idle"
            size={180}
            primaryColor={primaryColorArgb}
            secondaryColor={secondaryColorArgb}
          />
        </div>
      </div>

      {/* ── Color picker — intentional bespoke swatch grid UI ────── */}
      <div>
        <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint mb-2 px-1">
          {t('settings.mascot.colorHeading')}
        </h3>
        <div className="bg-surface rounded-xl border border-line overflow-hidden">
          {available.length === 0 ? (
            <p className="p-4 text-sm text-content-muted">{t('settings.mascot.noColorVariants')}</p>
          ) : (
            <div
              className="grid grid-cols-5 gap-3 p-4"
              role="radiogroup"
              aria-label={t('settings.mascot.colorAria')}>
              {available.map(opt => {
                const palette = getMascotPalette(opt.id);
                const selected = opt.id === activeColor;
                const label = t(opt.labelKey);
                return (
                  <button
                    key={opt.id}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    aria-label={label}
                    onClick={() => handleSelect(opt.id)}
                    data-testid={`mascot-color-${opt.id}`}
                    className={`flex flex-col items-center gap-2 rounded-lg p-2 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 ${
                      selected ? 'bg-surface-subtle' : 'hover:bg-surface-hover'
                    }`}>
                    <span
                      className={`w-10 h-10 rounded-full border-2 transition-shadow ${
                        selected ? 'border-primary-500 shadow-soft' : 'border-line'
                      }`}
                      style={
                        opt.id === 'custom'
                          ? {
                              background: `linear-gradient(135deg, ${customPrimary} 50%, ${customSecondary} 50%)`,
                            }
                          : { backgroundColor: palette.bodyFill }
                      }
                    />
                    <span className="text-xs text-content-secondary">{label}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
        {activeColor === 'custom' && (
          <div className="mt-3 bg-surface rounded-xl border border-line p-4 space-y-3">
            <label className="flex items-center gap-3">
              <input
                type="color"
                value={customPrimary}
                onChange={e => dispatch(setCustomPrimaryColor(e.target.value))}
                className="w-8 h-8 rounded-md border border-line dark:border-line-strong cursor-pointer p-0"
              />
              <span className="text-sm text-content-secondary">
                {t('settings.mascot.primaryColor')}
              </span>
              <code className="ml-auto text-[11px] font-mono text-content-faint">
                {customPrimary}
              </code>
            </label>
            <label className="flex items-center gap-3">
              <input
                type="color"
                value={customSecondary}
                onChange={e => dispatch(setCustomSecondaryColor(e.target.value))}
                className="w-8 h-8 rounded-md border border-line dark:border-line-strong cursor-pointer p-0"
              />
              <span className="text-sm text-content-secondary">
                {t('settings.mascot.secondaryColor')}
              </span>
              <code className="ml-auto text-[11px] font-mono text-content-faint">
                {customSecondary}
              </code>
            </label>
          </div>
        )}
        <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
          {t('settings.mascot.colorDesc')}
        </p>
      </div>

      {/* ── Voice picker section ──────────────────────────────────── */}
      <div>
        <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint mb-2 px-1">
          {t('settings.mascot.voice.heading')}
        </h3>
        <div className="bg-surface rounded-xl border border-line p-4 space-y-4">
          {/* Gender radio buttons — intentional bespoke pill UI */}
          <div
            role="radiogroup"
            aria-label={t('settings.mascot.voice.genderHeading')}
            className="space-y-1">
            <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
              {t('settings.mascot.voice.genderHeading')}
            </span>
            <div className="flex gap-2 pt-1">
              {(['female', 'male'] as const).map(g => (
                <button
                  key={g}
                  type="button"
                  role="radio"
                  aria-checked={voiceGender === g}
                  data-testid={`mascot-voice-gender-${g}`}
                  onClick={() => onGenderChange(g)}
                  className={`px-3 py-1.5 text-xs rounded-md border transition-colors ${
                    voiceGender === g
                      ? 'border-primary-500 bg-primary-50 dark:bg-primary-500/20 text-primary-700 dark:text-primary-200'
                      : 'border-line text-content-secondary hover:border-line-strong dark:hover:border-line-strong'
                  }`}>
                  {t(
                    g === 'female'
                      ? 'settings.mascot.voice.genderFemale'
                      : 'settings.mascot.voice.genderMale'
                  )}
                </button>
              ))}
            </div>
          </div>

          {/* Locale default checkbox — bespoke inline label layout */}
          <label className="flex items-start gap-2 text-sm text-content-secondary cursor-pointer">
            <input
              type="checkbox"
              data-testid="mascot-voice-locale-default"
              checked={useLocaleDefault}
              onChange={e => onLocaleDefaultToggle(e.target.checked)}
              className="mt-0.5 h-4 w-4 rounded border-line-strong text-primary-600 focus:ring-primary-500"
            />
            <span className="flex flex-col">
              <span>{t('settings.mascot.voice.useLocaleDefault')}</span>
              <span className="text-[11px] text-content-muted">
                {t('settings.mascot.voice.useLocaleDefaultDesc')}{' '}
                <code className="font-mono">{locale}</code> →{' '}
                <code className="font-mono">{localeDefaultVoiceId}</code>
              </span>
            </span>
          </label>

          {/* Preset dropdown — bespoke label + select combo */}
          <label className={`block space-y-1 ${presetPickerDisabled ? 'opacity-50' : ''}`}>
            <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
              {t('settings.mascot.voice.presetHeading')}
            </span>
            <SettingsSelect
              aria-label={t('settings.mascot.voice.presetHeading')}
              data-testid="mascot-voice-select"
              disabled={presetPickerDisabled}
              value={isCustomVoice ? '__custom__' : effectiveVoiceId}
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
                  aria-label={t('settings.mascot.voice.customHeading')}
                  data-testid="mascot-voice-input"
                  value={voiceDraft}
                  placeholder={t('settings.mascot.voice.customPlaceholder')}
                  onChange={e => setVoiceDraft(e.target.value)}
                  className="flex-1"
                />
                <Button
                  type="button"
                  variant="primary"
                  size="xs"
                  data-testid="mascot-voice-save-paste"
                  onClick={onSavePaste}
                  disabled={voiceDraft.trim() === (storedVoiceId ?? '').trim()}>
                  {t('common.save')}
                </Button>
              </div>
              <p className="text-[11px] text-content-muted">
                {t('settings.mascot.voice.customDesc')}
              </p>
            </label>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="primary"
              size="xs"
              data-testid="mascot-voice-preview"
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
              data-testid="mascot-voice-reset"
              onClick={onVoiceReset}
              disabled={storedVoiceId == null}>
              {t('settings.mascot.voice.reset')}
            </Button>
            <span
              data-testid="mascot-voice-current"
              className="ml-1 text-[11px] text-content-muted truncate max-w-[18rem]"
              title={effectiveVoiceId}>
              {t('settings.mascot.voice.current')}:{' '}
              <code className="font-mono">{effectiveVoiceId}</code>
            </span>
          </div>

          {voicePreviewError && (
            <div
              data-testid="mascot-voice-preview-error"
              className="rounded-md border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">
              {t('settings.mascot.voice.previewError')}: {voicePreviewError}
            </div>
          )}
        </div>
        <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
          {t('settings.mascot.voice.desc')}
        </p>
      </div>

      {/* ── Character picker — intentional bespoke list UI ────────── */}
      <div>
        <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint mb-2 px-1">
          {t('settings.mascot.characterHeading')}
        </h3>

        {/* Custom GIF input */}
        <div className="mb-3 bg-surface rounded-xl border border-line p-4 space-y-3">
          <label className="block space-y-1">
            <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
              {t('settings.mascot.customGifHeading')}
            </span>
            <div className="flex gap-2">
              <SettingsTextField
                aria-label={t('settings.mascot.customGifLabel')}
                data-testid="mascot-custom-gif-input"
                value={customGifDraft}
                placeholder={t('settings.mascot.customGifPlaceholder')}
                onChange={e => {
                  setCustomGifDraft(e.target.value);
                  setCustomGifError(null);
                }}
                className="flex-1"
              />
              <Button
                type="button"
                variant="primary"
                size="xs"
                data-testid="mascot-custom-gif-save"
                onClick={onSaveCustomGif}
                disabled={customGifDraft.trim() === (customMascotGifUrl ?? '').trim()}>
                {t('common.save')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="xs"
                data-testid="mascot-custom-gif-reset"
                onClick={onResetCustomGif}
                disabled={customMascotGifUrl == null && customGifDraft.trim().length === 0}>
                {t('common.reset')}
              </Button>
            </div>
          </label>
          {customGifError && (
            <p
              data-testid="mascot-custom-gif-error"
              className="text-xs text-coral-700 dark:text-coral-300">
              {customGifError}
            </p>
          )}
          {customMascotGifUrl && (
            <div className="flex justify-center rounded-lg border border-line-subtle bg-surface-muted p-3">
              <div style={{ width: 128, height: 128 }}>
                <CustomGifMascot src={customMascotGifUrl} />
              </div>
            </div>
          )}
        </div>

        {/* Mascot manifest library (tinyhumansai/mascots) */}
        <div className="bg-surface rounded-xl border border-line overflow-hidden">
          {manifestError && (
            <p className="p-4 text-sm text-coral-700 dark:text-coral-300">
              {t('settings.mascot.libraryUnavailable')}: {manifestError.message}
            </p>
          )}
          {!manifestError && manifestLoading && (
            <p className="p-4 text-sm text-content-muted">{t('settings.mascot.loadingLibrary')}</p>
          )}
          {manifest && manifest.mascots.length === 0 && !manifestError && (
            <p className="p-4 text-sm text-content-muted">{t('settings.mascot.noCharacters')}</p>
          )}
          {manifest && manifest.mascots.length > 0 && (
            <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
              <li>
                <button
                  type="button"
                  onClick={() => handleSelectMascot(null)}
                  aria-pressed={selectedMascotId == null && customMascotGifUrl == null}
                  className={`flex w-full items-center justify-between px-4 py-3 text-left text-sm hover:bg-surface-hover ${
                    selectedMascotId == null && customMascotGifUrl == null
                      ? 'bg-surface-muted font-medium'
                      : ''
                  }`}>
                  <span>{t('settings.mascot.localDefault')}</span>
                  {selectedMascotId == null && customMascotGifUrl == null && (
                    <span className="text-[10px] uppercase text-primary-600 dark:text-primary-300">
                      {t('settings.mascot.active')}
                    </span>
                  )}
                </button>
              </li>
              {manifest.mascots.map(mascot => {
                const active = mascot.id === selectedMascotId;
                const poseCount = new Set([
                  ...mascot.stateEngine.idlePoseCycle,
                  ...Object.values(mascot.stateEngine.states),
                ]).size;
                const visemeCount = mascot.stateEngine.visemeCodes.length;
                return (
                  <li key={mascot.id}>
                    <button
                      type="button"
                      onClick={() => handleSelectMascot(mascot.id)}
                      aria-pressed={active}
                      data-testid={`manifest-mascot-${mascot.id}`}
                      className={`flex w-full items-center justify-between px-4 py-3 text-left text-sm hover:bg-surface-hover ${
                        active ? 'bg-surface-muted font-medium' : ''
                      }`}>
                      <span className="flex flex-col">
                        <span className="flex items-center gap-2">
                          {mascot.name}
                          {mascot.status === 'draft' && (
                            <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wide text-amber-700 dark:bg-amber-500/20 dark:text-amber-200">
                              {t('settings.mascot.characterDraft')}
                            </span>
                          )}
                        </span>
                        <span className="text-[10px] text-content-muted">
                          {poseCount} {t('settings.mascot.characterStates')} · {visemeCount}{' '}
                          {t('settings.mascot.characterVisemes')}
                        </span>
                      </span>
                      {active && (
                        <span className="text-[10px] uppercase text-primary-600 dark:text-primary-300">
                          {t('settings.mascot.active')}
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        {activeEntry && !customMascotGifUrl && (
          <div className="mt-3 rounded-xl border border-line bg-surface-muted p-4">
            <p className="text-[11px] font-medium uppercase tracking-wide text-content-muted mb-2">
              {t('settings.mascot.characterPreview')} · {activeEntry.name}
            </p>
            <div className="flex justify-center">
              <div style={{ width: 160, height: 160 }}>
                <ManifestRiveMascot
                  key={activeEntry.id}
                  entry={activeEntry}
                  size={160}
                  primaryColor={primaryColorArgb}
                  secondaryColor={secondaryColorArgb}
                  idlePoseRotation
                />
              </div>
            </div>
          </div>
        )}
        <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
          {t('settings.mascot.characterDesc')}
        </p>
      </div>

      {/* ── Meeting duo: second mascot + per-mascot voices (issue #4277) ─
          Only meaningful for manifest mascots — a custom GIF avatar is a
          single-figure path, so the whole block hides while one is set. */}
      {manifest && manifest.mascots.length > 0 && !customMascotGifUrl && (
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint mb-2 px-1">
            {t('settings.mascot.secondaryHeading')}
          </h3>

          {/* Second-mascot picker — bespoke label + select combo mirroring
              the voice preset dropdown. The primary's id is disabled so the
              duo can never duplicate a single mascot. */}
          <div className="bg-surface rounded-xl border border-line p-4 space-y-1">
            <label className="block space-y-1">
              <span className="sr-only">{t('settings.mascot.secondaryHeading')}</span>
              <SettingsSelect
                aria-label={t('settings.mascot.secondaryHeading')}
                data-testid="mascot-secondary-select"
                value={secondaryMascotId ?? '__none__'}
                onChange={e =>
                  handleSelectSecondaryMascot(e.target.value === '__none__' ? null : e.target.value)
                }
                className="w-full">
                <option value="__none__">{t('settings.mascot.secondaryNone')}</option>
                {manifest.mascots.map(mascot => (
                  <option
                    key={mascot.id}
                    value={mascot.id}
                    // Skip the primary's id — it already speaks as the first
                    // mascot, so offering it as the second is a no-op the
                    // reducer would reject anyway. When the primary is still the
                    // default (selectedMascotId is null), the effective primary
                    // is the resolved default entry (activeEntry), so disable
                    // that too — otherwise the same mascot could be picked for
                    // both slots and the meeting would render two identical ones.
                    disabled={mascot.id === (selectedMascotId ?? activeEntry?.id)}>
                    {mascot.name}
                  </option>
                ))}
              </SettingsSelect>
            </label>
          </div>
          <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
            {t('settings.mascot.secondaryDesc')}
          </p>

          {/* Per-mascot voices — a row per mascot whose voice is actually
              addressable in `mascotVoices` (keyed by a concrete manifest
              id). The join path (`selectMeetingMascotVoicePair`) resolves
              the primary slot's voice from `mascotVoices[selectedMascotId]`,
              so the primary row only appears once a specific primary mascot
              is pinned; on the default mascot the effective single voice
              (governed by the Voice section above) is what plays. Each row
              writes its own `mascotVoices` entry and owns a guarded preview. */}
          {secondaryMascotId != null && secondaryMascotId !== selectedMascotId && (
            <div className="mt-3 space-y-3">
              <h4 className="text-[11px] font-medium uppercase tracking-wide text-content-muted px-1">
                {t('settings.mascot.perMascotVoiceHeading')}
              </h4>
              {selectedMascotId != null && (
                <PerMascotVoiceRow
                  mascotId={selectedMascotId}
                  label={t('settings.mascot.primaryVoiceLabel')}
                  testIdPrefix="mascot-voice-primary"
                />
              )}
              <PerMascotVoiceRow
                mascotId={secondaryMascotId}
                label={t('settings.mascot.secondaryVoiceLabel')}
                testIdPrefix="mascot-voice-secondary"
              />
            </div>
          )}
        </div>
      )}
    </>
  );

  // Embedded inside the tabbed Personality & Face page: the parent owns the
  // header, so render just the padded body.
  if (embedded) return <div className="p-4 space-y-5">{body}</div>;

  return <SettingsPanel>{body}</SettingsPanel>;
};

export default MascotPanel;
