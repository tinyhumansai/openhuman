import { useEffect, useMemo, useRef, useState } from 'react';

import { CustomGifMascot, ManifestRiveMascot, RiveMascot } from '../../../features/human/Mascot';
import { useMascotManifest } from '../../../features/human/Mascot/manifest/useMascotManifest';
import {
  getMascotPalette,
  hexToArgbInt,
  type MascotColor,
} from '../../../features/human/Mascot/mascotPalette';
import { synthesizeSpeech } from '../../../features/human/voice/ttsClient';
import { fileToDataUri, isAllowedMimeType } from '../../../lib/attachments';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  DEFAULT_MASCOT_COLOR,
  isCustomMascotGifUrl,
  type MascotVoiceGender,
  MAX_CUSTOM_MASCOT_AVATAR_UPLOAD_BYTES,
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
import { Alert, AlertDescription, Badge, Button, Card, Checkbox } from '../../ui';
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
  // An uploaded avatar is stored as a base64 data URL on the same field the URL
  // box writes. That string isn't meaningful or editable as text, so keep the
  // URL box blank for it rather than dumping ~2 MB of base64 into the input on
  // mount (issue #5360). Clearing an uploaded avatar is done via Reset.
  const storedIsUploadedAvatar = customMascotGifUrl?.startsWith('data:') ?? false;
  const [customGifDraft, setCustomGifDraft] = useState<string>(
    storedIsUploadedAvatar ? '' : (customMascotGifUrl ?? '')
  );
  const [customGifError, setCustomGifError] = useState<string | null>(null);
  // Hidden <input type="file"> driven by the "Upload image" button, so the
  // button can reuse the shared <Button> styling instead of a bare file input.
  const avatarFileInputRef = useRef<HTMLInputElement | null>(null);
  // Monotonically-bumped avatar-write id, same shape as `previewRequestIdRef`
  // below. Reading a file is async, so a user who picks an image and then hits
  // Reset / Save / picks a manifest mascot before the read finishes would have
  // the slower read land *after* their newer choice — restoring the upload and
  // wiping the mascot selection the reducer had just applied. Every avatar
  // write bumps this, and a resolved read bails if it is no longer current.
  const avatarWriteIdRef = useRef(0);

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
    avatarWriteIdRef.current += 1;
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
    avatarWriteIdRef.current += 1;
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
    avatarWriteIdRef.current += 1;
    setCustomGifDraft('');
    setCustomGifError(null);
    dispatch(setCustomMascotGifUrl(null));
  };

  // Upload a local image (PNG/GIF/JPEG/WebP/BMP) as the custom avatar (issue
  // #5360). The file is inlined as a base64 data URL and stored on the same
  // `customMascotGifUrl` field the URL box writes, so the render path is
  // unchanged. Type + size are checked *before* dispatch: an oversize blob
  // would silently fail to persist (localStorage quota is swallowed) and take
  // the rest of the mascot slice with it, so we reject it with a visible error.
  const onUploadAvatarFile = async (file: File | undefined) => {
    if (!file) return;
    if (!isAllowedMimeType(file.type)) {
      console.debug('[mascot-avatar] rejected upload: unsupported type', file.type);
      setCustomGifError(t('settings.mascot.customGifInvalidType'));
      return;
    }
    if (file.size > MAX_CUSTOM_MASCOT_AVATAR_UPLOAD_BYTES) {
      console.debug('[mascot-avatar] rejected upload: too large', file.size);
      setCustomGifError(t('settings.mascot.customGifTooLarge'));
      return;
    }
    avatarWriteIdRef.current += 1;
    const writeId = avatarWriteIdRef.current;
    try {
      const dataUri = await fileToDataUri(file);
      // Anything the user did while the read was in flight (Reset, Save, or
      // picking a manifest mascot) bumped the id and wins — dropping this
      // result is the whole point, so it is not an error path.
      if (writeId !== avatarWriteIdRef.current) {
        console.debug('[mascot-avatar] upload superseded, discarding', file.type, file.size);
        return;
      }
      console.debug('[mascot-avatar] upload accepted', file.type, file.size);
      setCustomGifError(null);
      setCustomGifDraft('');
      dispatch(setCustomMascotGifUrl(dataUri));
    } catch {
      // The read error carries `File.name`, which can be personal — log the
      // failure as a fixed event and keep the filename out of diagnostics.
      console.debug('[mascot-avatar] upload read failed', file.type, file.size);
      if (writeId !== avatarWriteIdRef.current) return;
      setCustomGifError(t('settings.mascot.customGifReadError'));
    }
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
        <Card divided={false}>
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
                    className={`flex flex-col items-center gap-2 rounded-lg p-2 transition-colors focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 ${
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
        </Card>
        {activeColor === 'custom' && (
          <Card padded divided={false} className="mt-3">
            <div className="space-y-3">
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
          </Card>
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
        <Card padded divided={false}>
          <div className="space-y-4">
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
              <Checkbox
                data-testid="mascot-voice-locale-default"
                checked={useLocaleDefault}
                onCheckedChange={onLocaleDefaultToggle}
                className="mt-0.5"
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
                className="bg-sage-500 hover:bg-sage-600 dark:hover:bg-sage-400">
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
              <Alert variant="warning" density="compact" data-testid="mascot-voice-preview-error">
                <AlertDescription>
                  {t('settings.mascot.voice.previewError')}: {voicePreviewError}
                </AlertDescription>
              </Alert>
            )}
          </div>
        </Card>
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
        <Card padded divided={false} className="mb-3">
          <div className="space-y-3">
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
                  disabled={
                    // For an uploaded (data-URL) avatar the box is intentionally
                    // blank, so an empty box means "no URL change", not "clear"
                    // (that would drop the upload). Only a typed URL enables Save.
                    storedIsUploadedAvatar
                      ? customGifDraft.trim().length === 0
                      : customGifDraft.trim() === (customMascotGifUrl ?? '').trim()
                  }>
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
            {/* Upload a local image file (issue #5360). The hidden input is
              driven by the styled button; its value is cleared after each pick
              so choosing the same file twice still fires onChange.

              `accept` is the `image/*` wildcard rather than an explicit type
              list. This app runs on CEF, whose built-in file-dialog runner
              (there is no CefDialogHandler in the shell) does not expand an
              enumerated accept list into selectable macOS file types: with
              either `image/png,image/jpeg,…` or those MIMEs paired with
              `.png,.jpg,…`, the native panel left every non-PNG image greyed
              out and unselectable. The wildcard goes through CEF's
              mime-table expansion instead and offers every known image type.

              The widened picker is not a widened contract: `isAllowedMimeType`
              still gates the read, so a type outside the allowlist (SVG, most
              importantly — it can carry inline scripts) is rejected with a
              visible error rather than silently accepted. */}
            <div className="flex items-center gap-2">
              <input
                ref={avatarFileInputRef}
                type="file"
                accept="image/*"
                className="sr-only"
                data-testid="mascot-custom-image-input"
                aria-label={t('settings.mascot.customGifUpload')}
                onChange={e => {
                  void onUploadAvatarFile(e.target.files?.[0]);
                  e.target.value = '';
                }}
              />
              <Button
                type="button"
                variant="secondary"
                size="xs"
                data-testid="mascot-custom-image-upload"
                onClick={() => avatarFileInputRef.current?.click()}>
                {t('settings.mascot.customGifUpload')}
              </Button>
            </div>
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
        </Card>

        {/* Mascot manifest library (tinyhumansai/mascots) */}
        <Card divided={false}>
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
            <ul className="divide-y divide-line-subtle">
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
                            <Badge variant="warning">{t('settings.mascot.characterDraft')}</Badge>
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
        </Card>

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
          <Card padded divided={false}>
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
          </Card>
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
  // header AND the page gutter (`SettingsPanel` supplies `p-4` now), so this
  // renders the body flush — a `p-4` here would indent it twice.
  if (embedded) return <div className="space-y-5">{body}</div>;

  return <SettingsPanel>{body}</SettingsPanel>;
};

export default MascotPanel;
