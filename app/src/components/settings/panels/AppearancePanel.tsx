import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  FONT_SIZE_PX,
  type FontSize,
  MAX_FONT_SIZE_PX,
  MIN_FONT_SIZE_PX,
  selectEffectiveFontSizePx,
  setCustomFontSizePx,
  setFontSize,
} from '../../../store/themeSlice';
import LanguageSelect from '../../LanguageSelect';
import { Card } from '../../ui';
import Slider from '../../ui/Slider';
import { SettingsNumberField, SettingsRow, SettingsSection } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import ThemeStudioPanel from './ThemeStudioPanel';

interface FontSizeOption {
  id: FontSize;
  label: string;
  description: string;
  /** Sample "A" glyph sized to preview the option inline. */
  glyphClass: string;
}

const AppearancePanel = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const effectiveFontSizePx = useAppSelector(selectEffectiveFontSizePx);

  // Local draft for the numeric px field so partial typing doesn't thrash the
  // store; commits (blur / Enter) clamp and dispatch, while the slider dispatches
  // live. Re-sync the draft to the effective size when it changes externally
  // (slider drag, preset click) via React's render-phase pattern — no effect,
  // so there's no cascading-render round-trip.
  const [pxDraft, setPxDraft] = useState(String(effectiveFontSizePx));
  const [syncedPx, setSyncedPx] = useState(effectiveFontSizePx);
  if (effectiveFontSizePx !== syncedPx) {
    setSyncedPx(effectiveFontSizePx);
    setPxDraft(String(effectiveFontSizePx));
  }
  const commitCustomFontSize = () => {
    const parsed = Number.parseInt(pxDraft, 10);
    if (Number.isFinite(parsed)) {
      console.debug('[appearance] commit custom font-size', { pxDraft, parsed });
      dispatch(setCustomFontSizePx(parsed));
    } else {
      console.debug('[appearance] custom font-size rejected, reverting draft', { pxDraft });
      setPxDraft(String(effectiveFontSizePx));
    }
  };
  const handleFontSizeSlider = (values: number[]) => {
    const px = values[0];
    console.debug('[appearance] custom font-size slider', { px });
    dispatch(setCustomFontSizePx(px));
  };

  // Built at render time so the labels follow the active locale; `t()` itself
  // memoises on locale change, so this stays stable across re-renders within a
  // locale.
  const FONT_SIZE_OPTIONS: FontSizeOption[] = [
    {
      id: 'small',
      label: t('settings.appearance.fontSizeSmall'),
      description: t('settings.appearance.fontSizeSmallDesc'),
      glyphClass: 'text-xs',
    },
    {
      id: 'medium',
      label: t('settings.appearance.fontSizeMedium'),
      description: t('settings.appearance.fontSizeMediumDesc'),
      glyphClass: 'text-sm',
    },
    {
      id: 'large',
      label: t('settings.appearance.fontSizeLarge'),
      description: t('settings.appearance.fontSizeLargeDesc'),
      glyphClass: 'text-base',
    },
    {
      id: 'xlarge',
      label: t('settings.appearance.fontSizeXLarge'),
      description: t('settings.appearance.fontSizeXLargeDesc'),
      glyphClass: 'text-lg',
    },
  ];

  return (
    <SettingsPanel description={t('settings.appearance.menuDesc')}>
      {/* Colours, fonts and background — the former "Theme studio" page, now
          sections of this one. Its gallery header carries the Light / Dark /
          Auto toggle, which is why this page no longer has a separate theme-mode
          tile list: `setThemeMode` and `setThemeVariant` write the same two
          slice fields, so the two controls were one control shown twice. */}
      <ThemeStudioPanel embedded />

      {/* ── Font size picker — intentional bespoke tile UI ─────────── */}
      <div>
        <h3 className="mb-2 px-1 font-title text-sm font-semibold text-content">
          {t('settings.appearance.fontSizeHeading')}
        </h3>
        {/* Card forwards no `role`/`aria-label` to its wrapper, so the
            radiogroup semantics move to an inner div that still encloses
            every option — `within(group)` in the specs resolves the same. */}
        <Card divided={false}>
          <div role="radiogroup" aria-label={t('settings.appearance.fontSizeAria')}>
            {FONT_SIZE_OPTIONS.map((opt, idx) => {
              // Highlight the preset whose px matches the effective size, so a
              // fine-tuned value landing exactly on a preset still lights it up.
              const selected = Number.parseInt(FONT_SIZE_PX[opt.id], 10) === effectiveFontSizePx;
              return (
                <button
                  key={opt.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => dispatch(setFontSize(opt.id))}
                  className={`w-full flex items-center gap-3 px-4 py-3 text-left transition-colors focus:outline-hidden focus-visible:bg-primary-50 dark:focus-visible:bg-primary-900/30 ${
                    idx !== 0 ? 'border-t border-line-subtle' : ''
                  } ${selected ? 'bg-primary-50 dark:bg-primary-500/10' : 'hover:bg-surface-hover'}`}>
                  <span
                    className={`flex items-center justify-center w-9 h-9 rounded-lg ${
                      selected
                        ? 'bg-primary-500 text-content-inverted'
                        : 'bg-surface-subtle text-content-secondary'
                    }`}>
                    <span className={`font-semibold leading-none ${opt.glyphClass}`} aria-hidden>
                      A
                    </span>
                  </span>
                  <span className="flex-1 min-w-0">
                    <span className="block text-sm font-medium text-content">{opt.label}</span>
                    <span className="block text-xs text-content-muted">{opt.description}</span>
                  </span>
                  {selected && (
                    <svg
                      className="w-5 h-5 text-primary-500"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                      aria-hidden>
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M5 13l4 4L19 7"
                      />
                    </svg>
                  )}
                </button>
              );
            })}
          </div>
        </Card>
        {/* Fine-tune the exact size beyond the presets (issue #4246). */}
        {/* px-4 py-3, not Card's `padded` p-4, to keep this box's original
            (denser) vertical padding pixel-identical. */}
        <Card divided={false} className="px-4 py-3 mt-3">
          <div className="flex items-center justify-between gap-3">
            <label htmlFor="font-size-custom-number" className="text-sm font-medium text-content">
              {t('settings.appearance.fontSizeCustomLabel')}
            </label>
            <SettingsNumberField
              id="font-size-custom-number"
              value={pxDraft}
              onChange={setPxDraft}
              onCommit={commitCustomFontSize}
              unit={t('settings.appearance.fontSizeUnit')}
              min={MIN_FONT_SIZE_PX}
              max={MAX_FONT_SIZE_PX}
              aria-label={t('settings.appearance.fontSizeCustomAria')}
              data-testid="font-size-custom-number"
            />
          </div>
          <Slider
            id="font-size-slider"
            min={MIN_FONT_SIZE_PX}
            max={MAX_FONT_SIZE_PX}
            step={1}
            value={[effectiveFontSizePx]}
            onValueChange={handleFontSizeSlider}
            thumbLabels={[t('settings.appearance.fontSizeCustomSliderAria')]}
            aria-valuetext={`${effectiveFontSizePx}${t('settings.appearance.fontSizeUnit')}`}
            className="mt-3"
            data-testid="font-size-slider"
          />
          <div className="flex items-center justify-between mt-1 text-[11px] text-content-faint">
            <span>{`${MIN_FONT_SIZE_PX}${t('settings.appearance.fontSizeUnit')}`}</span>
            <span>{`${MAX_FONT_SIZE_PX}${t('settings.appearance.fontSizeUnit')}`}</span>
          </div>
        </Card>

        <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
          {t('settings.appearance.fontSizeHelperText')}
        </p>
      </div>

      {/* ── Display language (moved from the old settings home list) ── */}
      <SettingsSection title={t('settings.language')}>
        <SettingsRow
          label={t('settings.language')}
          description={t('settings.languageDesc')}
          control={<LanguageSelect ariaLabel={t('settings.language')} />}
        />
      </SettingsSection>
    </SettingsPanel>
  );
};

export default AppearancePanel;
