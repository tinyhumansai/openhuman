import { useState } from 'react';
import { Navigate, useLocation } from 'react-router-dom';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { selectChatMascotDismissed, setChatMascotDismissed } from '../../../store/mascotSlice';
import {
  FONT_SIZE_PX,
  type FontSize,
  MAX_FONT_SIZE_PX,
  MIN_FONT_SIZE_PX,
  selectEffectiveFontSizePx,
  setCustomFontSizePx,
  setFontSize,
} from '../../../store/themeSlice';
import { Card, Slider, Switch } from '../../ui';
import { SettingsNumberField } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import LayoutSettings from './theme/LayoutSettings';

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
  const chatMascotDismissed = useAppSelector(selectChatMascotDismissed);
  const location = useLocation();

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

  const body = (
    <>
      <Card
        title={t('settings.appearance.fontSizeHeading')}
        description={t('settings.appearance.fontSizeHelperText')}
        data-testid="font-size-card">
        {/* Card forwards no `role`/`aria-label` to its wrapper, so the
              radiogroup semantics sit on an inner div that encloses every
              option — `within(group)` in the specs resolves the same. */}
        <div
          role="radiogroup"
          aria-label={t('settings.appearance.fontSizeAria')}
          className="grid grid-cols-2 gap-2 p-4 sm:grid-cols-4">
          {FONT_SIZE_OPTIONS.map(opt => {
            // Highlight the preset whose px matches the effective size, so a
            // fine-tuned value landing exactly on a preset still lights it up.
            const selected = Number.parseInt(FONT_SIZE_PX[opt.id], 10) === effectiveFontSizePx;
            return (
              <button
                key={opt.id}
                type="button"
                role="radio"
                aria-checked={selected}
                title={opt.description}
                onClick={() => dispatch(setFontSize(opt.id))}
                className={cn(
                  'flex flex-col items-center justify-center gap-2 rounded-xl border px-3 py-4 transition-colors',
                  'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25',
                  selected
                    ? 'border-primary-500 bg-primary-50 ring-1 ring-primary-500 dark:bg-primary-500/10'
                    : 'border-line hover:bg-surface-hover'
                )}>
                <span
                  className={cn(
                    'flex h-8 items-center font-semibold leading-none',
                    opt.glyphClass,
                    selected ? 'text-primary-500' : 'text-content-secondary'
                  )}
                  aria-hidden>
                  Aa
                </span>
                <span className="text-xs font-medium text-content">{opt.label}</span>
              </button>
            );
          })}
        </div>

        {/* Fine-tune the exact size beyond the presets (issue #4246). */}
        <div className="px-4 py-3">
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
          <div className="mt-1 flex items-center justify-between text-[11px] text-content-faint">
            <span>{`${MIN_FONT_SIZE_PX}${t('settings.appearance.fontSizeUnit')}`}</span>
            <span>{`${MAX_FONT_SIZE_PX}${t('settings.appearance.fontSizeUnit')}`}</span>
          </div>
        </div>
      </Card>
    </>
  );

  // Theme Studio and Layout were briefly tabs here (`#studio`, `#layout`).
  if (location.hash === '#studio') return <Navigate to="/settings/theme" replace />;

  return (
    <SettingsPanel testId="appearance-panel" description={t('settings.appearance.menuDesc')}>
      {body}
      {/* Corner rounding, border contrast, and which areas draw borders. */}
      <LayoutSettings />
      <Card title={t('settings.appearance.chatHeading')}>
        <div className="flex items-center justify-between gap-3 p-4">
          <label htmlFor="show-chat-mascot" className="text-sm font-medium text-content">
            {t('settings.appearance.showChatMascot')}
          </label>
          <Switch
            id="show-chat-mascot"
            aria-label={t('settings.appearance.showChatMascot')}
            checked={!chatMascotDismissed}
            onCheckedChange={shown => dispatch(setChatMascotDismissed(!shown))}
          />
        </div>
      </Card>
    </SettingsPanel>
  );
};

export default AppearancePanel;
