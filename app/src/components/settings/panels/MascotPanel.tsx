import { useEffect, useMemo, useRef, useState } from 'react';

import { CustomGifMascot, RiveMascot } from '../../../features/human/Mascot';
import { BackendMascot } from '../../../features/human/Mascot/backend/BackendMascot';
import type { MascotDetail, MascotSummary } from '../../../features/human/Mascot/backend/types';
import {
  getMascotPalette,
  hexToArgbInt,
  type MascotColor,
} from '../../../features/human/Mascot/mascotPalette';
import { synthesizeSpeech } from '../../../features/human/voice/ttsClient';
import { useT } from '../../../lib/i18n/I18nContext';
import { fetchMascotList, getCachedMascotDetail } from '../../../services/mascotService';
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
  selectSelectedMascotId,
  setCustomMascotGifUrl,
  setCustomPrimaryColor,
  setCustomSecondaryColor,
  setMascotColor,
  setMascotVoiceGender,
  setMascotVoiceId,
  setMascotVoiceUseLocaleDefault,
  setSelectedMascotId,
  SUPPORTED_MASCOT_COLORS,
} from '../../../store/mascotSlice';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  defaultVoiceIdForLocale,
  ELEVENLABS_VOICE_PRESETS,
  isCuratedVoicePreset,
} from './elevenlabsVoicePresets';

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

const MascotPanel = () => {
  const { t, locale } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const storedColor = useAppSelector(selectMascotColor);
  const customPrimary = useAppSelector(selectCustomPrimaryColor);
  const customSecondary = useAppSelector(selectCustomSecondaryColor);
  const selectedMascotId = useAppSelector(selectSelectedMascotId);
  const customMascotGifUrl = useAppSelector(selectCustomMascotGifUrl);
  const storedVoiceId = useAppSelector(selectMascotVoiceId);
  const voiceGender = useAppSelector(selectMascotVoiceGender);
  const useLocaleDefault = useAppSelector(selectMascotVoiceUseLocaleDefault);
  const effectiveVoiceId = useAppSelector(selectEffectiveMascotVoiceId);

  // Backend mascot library (PR tinyhumansai/backend#770). The list endpoint
  // is cheap (no SVG bytes); per-id detail is fetched on demand so the
  // animated preview only pays for the active selection.
  const [backendList, setBackendList] = useState<MascotSummary[] | null>(null);
  const [backendListError, setBackendListError] = useState<string | null>(null);
  const [activeDetail, setActiveDetail] = useState<MascotDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
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

  useEffect(() => {
    let cancelled = false;
    fetchMascotList()
      .then(list => {
        if (cancelled) return;
        setBackendList(list);
        setBackendListError(null);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : t('settings.mascot.loadLibraryError');
        setBackendListError(message);
        setBackendList([]);
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  useEffect(() => {
    if (!selectedMascotId) return;
    let cancelled = false;
    getCachedMascotDetail(selectedMascotId)
      .then(detail => {
        if (cancelled) return;
        setActiveDetail(detail);
        setDetailError(null);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : t('settings.mascot.loadDetailError');
        setDetailError(message);
        setActiveDetail(null);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedMascotId, t]);

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

  const handleSelectBackend = (id: string | null) => {
    dispatch(setSelectedMascotId(id));
    setCustomGifError(null);
    if (id == null) {
      setCustomGifDraft('');
      dispatch(setCustomMascotGifUrl(null));
    } else {
      setCustomGifDraft('');
    }
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
  const visibleActiveDetail = selectedMascotId ? activeDetail : null;
  const visibleDetailError = selectedMascotId ? detailError : null;

  const activePalette = getMascotPalette(activeColor);
  const primaryColorArgb = useMemo(
    () => hexToArgbInt(activeColor === 'custom' ? customPrimary : activePalette.bodyFill),
    [activeColor, customPrimary, activePalette]
  );
  const secondaryColorArgb = useMemo(
    () => hexToArgbInt(activeColor === 'custom' ? customSecondary : activePalette.neckShadowColor),
    [activeColor, customSecondary, activePalette]
  );

  return (
    <div>
      <SettingsHeader
        title={t('settings.mascot.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
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

        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-2 px-1">
            {t('settings.mascot.colorHeading')}
          </h3>
          <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 overflow-hidden">
            {available.length === 0 ? (
              <p className="p-4 text-sm text-stone-500 dark:text-neutral-400">
                {t('settings.mascot.noColorVariants')}
              </p>
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
                        selected
                          ? 'bg-stone-100 dark:bg-neutral-800'
                          : 'hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60'
                      }`}>
                      <span
                        className={`w-10 h-10 rounded-full border-2 transition-shadow ${
                          selected
                            ? 'border-primary-500 shadow-soft'
                            : 'border-stone-200 dark:border-neutral-800'
                        }`}
                        style={
                          opt.id === 'custom'
                            ? {
                                background: `linear-gradient(135deg, ${customPrimary} 50%, ${customSecondary} 50%)`,
                              }
                            : { backgroundColor: palette.bodyFill }
                        }
                      />
                      <span className="text-xs text-stone-700 dark:text-neutral-200">{label}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          {activeColor === 'custom' && (
            <div className="mt-3 bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
              <label className="flex items-center gap-3">
                <input
                  type="color"
                  value={customPrimary}
                  onChange={e => dispatch(setCustomPrimaryColor(e.target.value))}
                  className="w-8 h-8 rounded-md border border-stone-200 dark:border-neutral-700 cursor-pointer p-0"
                />
                <span className="text-sm text-stone-700 dark:text-neutral-200">
                  {t('settings.mascot.primaryColor')}
                </span>
                <code className="ml-auto text-[11px] font-mono text-stone-400 dark:text-neutral-500">
                  {customPrimary}
                </code>
              </label>
              <label className="flex items-center gap-3">
                <input
                  type="color"
                  value={customSecondary}
                  onChange={e => dispatch(setCustomSecondaryColor(e.target.value))}
                  className="w-8 h-8 rounded-md border border-stone-200 dark:border-neutral-700 cursor-pointer p-0"
                />
                <span className="text-sm text-stone-700 dark:text-neutral-200">
                  {t('settings.mascot.secondaryColor')}
                </span>
                <code className="ml-auto text-[11px] font-mono text-stone-400 dark:text-neutral-500">
                  {customSecondary}
                </code>
              </label>
            </div>
          )}
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            {t('settings.mascot.colorDesc')}
          </p>
        </div>

        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-2 px-1">
            {t('settings.mascot.voice.heading')}
          </h3>
          <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-4">
            <div
              role="radiogroup"
              aria-label={t('settings.mascot.voice.genderHeading')}
              className="space-y-1">
              <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
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
                        : 'border-stone-200 dark:border-neutral-800 text-stone-700 dark:text-neutral-200 hover:border-stone-300 dark:hover:border-neutral-700'
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

            <label className="flex items-start gap-2 text-sm text-stone-700 dark:text-neutral-200 cursor-pointer">
              <input
                type="checkbox"
                data-testid="mascot-voice-locale-default"
                checked={useLocaleDefault}
                onChange={e => onLocaleDefaultToggle(e.target.checked)}
                className="mt-0.5 h-4 w-4 rounded border-stone-300 dark:border-neutral-700 text-primary-600 focus:ring-primary-500"
              />
              <span className="flex flex-col">
                <span>{t('settings.mascot.voice.useLocaleDefault')}</span>
                <span className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {t('settings.mascot.voice.useLocaleDefaultDesc')}{' '}
                  <code className="font-mono">{locale}</code> →{' '}
                  <code className="font-mono">{localeDefaultVoiceId}</code>
                </span>
              </span>
            </label>

            <label className={`block space-y-1 ${presetPickerDisabled ? 'opacity-50' : ''}`}>
              <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                {t('settings.mascot.voice.presetHeading')}
              </span>
              <select
                aria-label={t('settings.mascot.voice.presetHeading')}
                data-testid="mascot-voice-select"
                disabled={presetPickerDisabled}
                value={isCustomVoice ? '__custom__' : effectiveVoiceId}
                onChange={e => onPresetChange(e.target.value)}
                className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-primary-400 disabled:cursor-not-allowed">
                {visiblePresets.map(v => (
                  <option key={v.id} value={v.id}>
                    {v.label}
                  </option>
                ))}
                <option value="__custom__">{t('settings.mascot.voice.customOption')}</option>
              </select>
            </label>

            {isCustomVoice && (
              <label className="block space-y-1">
                <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                  {t('settings.mascot.voice.customHeading')}
                </span>
                <div className="flex gap-2">
                  <input
                    aria-label={t('settings.mascot.voice.customHeading')}
                    data-testid="mascot-voice-input"
                    value={voiceDraft}
                    placeholder={t('settings.mascot.voice.customPlaceholder')}
                    onChange={e => setVoiceDraft(e.target.value)}
                    className="flex-1 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                  <button
                    type="button"
                    data-testid="mascot-voice-save-paste"
                    onClick={onSavePaste}
                    disabled={voiceDraft.trim() === (storedVoiceId ?? '').trim()}
                    className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                    {t('common.save')}
                  </button>
                </div>
                <p className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {t('settings.mascot.voice.customDesc')}
                </p>
              </label>
            )}

            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                data-testid="mascot-voice-preview"
                onClick={() => void onVoicePreview()}
                disabled={isPreviewingVoice}
                className="px-3 py-1.5 text-xs rounded-md bg-emerald-600 hover:bg-emerald-700 disabled:opacity-60 text-white">
                {isPreviewingVoice
                  ? t('settings.mascot.voice.previewing')
                  : t('settings.mascot.voice.preview')}
              </button>
              <button
                type="button"
                data-testid="mascot-voice-reset"
                onClick={onVoiceReset}
                disabled={storedVoiceId == null}
                className="px-3 py-1.5 text-xs rounded-md border border-stone-300 dark:border-neutral-700 hover:border-stone-400 dark:hover:border-neutral-600 disabled:opacity-60 text-stone-700 dark:text-neutral-200">
                {t('settings.mascot.voice.reset')}
              </button>
              <span
                data-testid="mascot-voice-current"
                className="ml-1 text-[11px] text-stone-500 dark:text-neutral-400 truncate max-w-[18rem]"
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
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            {t('settings.mascot.voice.desc')}
          </p>
        </div>

        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-2 px-1">
            {t('settings.mascot.characterHeading')}
          </h3>
          <div className="mb-3 bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
            <label className="block space-y-1">
              <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                {t('settings.mascot.customGifHeading')}
              </span>
              <div className="flex gap-2">
                <input
                  aria-label={t('settings.mascot.customGifLabel')}
                  data-testid="mascot-custom-gif-input"
                  value={customGifDraft}
                  placeholder={t('settings.mascot.customGifPlaceholder')}
                  onChange={e => {
                    setCustomGifDraft(e.target.value);
                    setCustomGifError(null);
                  }}
                  className="flex-1 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
                />
                <button
                  type="button"
                  data-testid="mascot-custom-gif-save"
                  onClick={onSaveCustomGif}
                  disabled={customGifDraft.trim() === (customMascotGifUrl ?? '').trim()}
                  className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                  {t('common.save')}
                </button>
                <button
                  type="button"
                  data-testid="mascot-custom-gif-reset"
                  onClick={onResetCustomGif}
                  disabled={customMascotGifUrl == null && customGifDraft.trim().length === 0}
                  className="px-3 py-1.5 text-xs rounded-md border border-stone-300 dark:border-neutral-700 hover:border-stone-400 dark:hover:border-neutral-600 disabled:opacity-60 text-stone-700 dark:text-neutral-200">
                  {t('common.reset')}
                </button>
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
              <div className="flex justify-center rounded-lg border border-stone-100 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
                <div style={{ width: 128, height: 128 }}>
                  <CustomGifMascot src={customMascotGifUrl} />
                </div>
              </div>
            )}
          </div>
          <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 overflow-hidden">
            {backendListError && (
              <p className="p-4 text-sm text-coral-700 dark:text-coral-300">
                {t('settings.mascot.libraryUnavailable')}: {backendListError}
              </p>
            )}
            {!backendListError && backendList === null && (
              <p className="p-4 text-sm text-stone-500 dark:text-neutral-400">
                {t('settings.mascot.loadingLibrary')}
              </p>
            )}
            {backendList && backendList.length === 0 && !backendListError && (
              <p className="p-4 text-sm text-stone-500 dark:text-neutral-400">
                {t('settings.mascot.noCharacters')}
              </p>
            )}
            {backendList && backendList.length > 0 && (
              <ul className="divide-y divide-stone-100 dark:divide-neutral-800">
                <li>
                  <button
                    type="button"
                    onClick={() => handleSelectBackend(null)}
                    aria-pressed={selectedMascotId == null && customMascotGifUrl == null}
                    className={`flex w-full items-center justify-between px-4 py-3 text-left text-sm hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 ${
                      selectedMascotId == null && customMascotGifUrl == null
                        ? 'bg-stone-50 dark:bg-neutral-800/60 font-medium'
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
                {backendList.map(summary => {
                  const active = summary.id === selectedMascotId;
                  return (
                    <li key={summary.id}>
                      <button
                        type="button"
                        onClick={() => handleSelectBackend(summary.id)}
                        aria-pressed={active}
                        data-testid={`backend-mascot-${summary.id}`}
                        className={`flex w-full items-center justify-between px-4 py-3 text-left text-sm hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 ${
                          active ? 'bg-stone-50 dark:bg-neutral-800/60 font-medium' : ''
                        }`}>
                        <span className="flex flex-col">
                          <span>{summary.name}</span>
                          <span className="text-[10px] text-stone-500 dark:text-neutral-400">
                            v{summary.version} · {summary.states.length}{' '}
                            {t('settings.mascot.characterStates')}
                            {summary.hasVisemes
                              ? ` · ${t('settings.mascot.characterVisemes')}`
                              : ''}
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

          {visibleActiveDetail && (
            <div className="mt-3 rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
              <p className="text-[11px] font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400 mb-2">
                {t('settings.mascot.characterPreview')} · {visibleActiveDetail.name}
              </p>
              <div className="flex justify-center">
                <div style={{ width: 160, height: 160 }}>
                  <BackendMascot mascot={visibleActiveDetail} />
                </div>
              </div>
            </div>
          )}
          {visibleDetailError && (
            <p className="mt-2 text-xs text-coral-700 dark:text-coral-300 px-1">
              {visibleDetailError}
            </p>
          )}
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            {t('settings.mascot.characterDesc')}
          </p>
        </div>
      </div>
    </div>
  );
};

export default MascotPanel;
