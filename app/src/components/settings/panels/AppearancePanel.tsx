import { type ChangeEvent, type ReactElement, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  type AgentMessageViewMode,
  FONT_SIZE_PX,
  type FontSize,
  MAX_FONT_SIZE_PX,
  MIN_FONT_SIZE_PX,
  selectEffectiveFontSizePx,
  setAgentMessageViewMode,
  setCustomFontSizePx,
  setFontSize,
  setHideAgentInsights,
  setTabBarLabels,
  setThemeMode,
  type TabBarLabels,
  type ThemeMode,
} from '../../../store/themeSlice';
import LanguageSelect from '../../LanguageSelect';
import { SettingsNumberField, SettingsRow, SettingsSection, SettingsSwitch } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

interface ModeOption {
  id: ThemeMode;
  label: string;
  description: string;
  icon: ReactElement;
}

interface FontSizeOption {
  id: FontSize;
  label: string;
  description: string;
  /** Sample "A" glyph sized to preview the option inline. */
  glyphClass: string;
}

const SunIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden>
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M12 3v2m0 14v2m9-9h-2M5 12H3m15.364-6.364l-1.414 1.414M7.05 16.95l-1.414 1.414m12.728 0l-1.414-1.414M7.05 7.05L5.636 5.636M16 12a4 4 0 11-8 0 4 4 0 018 0z"
    />
  </svg>
);

const MoonIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden>
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z"
    />
  </svg>
);

const SystemIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden>
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M9 17v2m6-2v2m-9-2h12a2 2 0 002-2V7a2 2 0 00-2-2H6a2 2 0 00-2 2v8a2 2 0 002 2z"
    />
  </svg>
);

const AppearancePanel = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const mode = useAppSelector(state => state.theme.mode);
  const effectiveFontSizePx = useAppSelector(selectEffectiveFontSizePx);
  const tabBarLabels = useAppSelector(state => state.theme.tabBarLabels);
  const agentMessageViewMode = useAppSelector(
    state => state.theme.agentMessageViewMode ?? 'bubbles'
  );
  const hideAgentInsights = useAppSelector(state => state.theme.hideAgentInsights ?? false);
  const labelsAlwaysVisible = tabBarLabels === 'always';
  const assistantTextModeEnabled = agentMessageViewMode === 'text';
  const toggleTabBarLabels = () => {
    const next: TabBarLabels = labelsAlwaysVisible ? 'hover' : 'always';
    dispatch(setTabBarLabels(next));
  };
  const toggleAssistantTextMode = () => {
    const next: AgentMessageViewMode = assistantTextModeEnabled ? 'bubbles' : 'text';
    dispatch(setAgentMessageViewMode(next));
  };
  const toggleHideAgentInsights = () => {
    dispatch(setHideAgentInsights(!hideAgentInsights));
  };

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
  const handleFontSizeSlider = (event: ChangeEvent<HTMLInputElement>) => {
    const px = Number(event.target.value);
    console.debug('[appearance] custom font-size slider', { px });
    dispatch(setCustomFontSizePx(px));
  };

  // Build at render time so the labels follow the active locale; `t()` itself
  // memoises on locale change, so this stays stable across re-renders within a
  // locale.
  const OPTIONS: ModeOption[] = [
    {
      id: 'light',
      label: t('settings.appearance.modeLight'),
      description: t('settings.appearance.modeLightDesc'),
      icon: SunIcon,
    },
    {
      id: 'dark',
      label: t('settings.appearance.modeDark'),
      description: t('settings.appearance.modeDarkDesc'),
      icon: MoonIcon,
    },
    {
      id: 'system',
      label: t('settings.appearance.modeSystem'),
      description: t('settings.appearance.modeSystemDesc'),
      icon: SystemIcon,
    },
  ];

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
      {/* ── Theme picker — intentional bespoke tile UI ─────────────── */}
      <div>
        <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint mb-2 px-1">
          {t('settings.appearance.themeHeading')}
        </h3>
        <div
          className="bg-surface rounded-xl border border-line overflow-hidden"
          role="radiogroup"
          aria-label={t('settings.appearance.themeAria')}>
          {OPTIONS.map((opt, idx) => {
            const selected = opt.id === mode;
            return (
              <button
                key={opt.id}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => dispatch(setThemeMode(opt.id))}
                className={`w-full flex items-center gap-3 px-4 py-3 text-left transition-colors focus:outline-none focus-visible:bg-primary-50 dark:focus-visible:bg-primary-900/30 ${
                  idx !== 0 ? 'border-t border-line-subtle' : ''
                } ${selected ? 'bg-primary-50 dark:bg-primary-500/10' : 'hover:bg-surface-hover'}`}>
                <span
                  className={`flex items-center justify-center w-9 h-9 rounded-lg ${
                    selected
                      ? 'bg-primary-500 text-content-inverted'
                      : 'bg-surface-subtle text-content-secondary'
                  }`}>
                  {opt.icon}
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
        <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
          {t('settings.appearance.helperText')}
        </p>
      </div>

      {/* ── Font size picker — intentional bespoke tile UI ─────────── */}
      <div>
        <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint mb-2 px-1">
          {t('settings.appearance.fontSizeHeading')}
        </h3>
        <div
          className="bg-surface rounded-xl border border-line overflow-hidden"
          role="radiogroup"
          aria-label={t('settings.appearance.fontSizeAria')}>
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
                className={`w-full flex items-center gap-3 px-4 py-3 text-left transition-colors focus:outline-none focus-visible:bg-primary-50 dark:focus-visible:bg-primary-900/30 ${
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
        {/* Fine-tune the exact size beyond the presets (issue #4246). */}
        <div className="bg-surface rounded-xl border border-line px-4 py-3 mt-3">
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
          <input
            id="font-size-slider"
            type="range"
            min={MIN_FONT_SIZE_PX}
            max={MAX_FONT_SIZE_PX}
            step={1}
            value={effectiveFontSizePx}
            onChange={handleFontSizeSlider}
            aria-label={t('settings.appearance.fontSizeCustomSliderAria')}
            aria-valuetext={`${effectiveFontSizePx}${t('settings.appearance.fontSizeUnit')}`}
            className="w-full mt-3 accent-primary-500 cursor-pointer"
            data-testid="font-size-slider"
          />
          <div className="flex items-center justify-between mt-1 text-[11px] text-content-faint">
            <span>{`${MIN_FONT_SIZE_PX}${t('settings.appearance.fontSizeUnit')}`}</span>
            <span>{`${MAX_FONT_SIZE_PX}${t('settings.appearance.fontSizeUnit')}`}</span>
          </div>
        </div>

        <p className="text-xs text-content-muted leading-relaxed px-1 mt-2">
          {t('settings.appearance.fontSizeHelperText')}
        </p>
      </div>

      {/* ── Tab bar labels toggle ──────────────────────────────────── */}
      <SettingsSection title={t('settings.appearance.tabBarHeading')}>
        <SettingsRow
          htmlFor="switch-tab-bar-labels"
          label={t('settings.appearance.tabBarAlwaysShowLabels')}
          description={t('settings.appearance.tabBarAlwaysShowLabelsDesc')}
          control={
            <SettingsSwitch
              id="switch-tab-bar-labels"
              checked={labelsAlwaysVisible}
              onCheckedChange={toggleTabBarLabels}
              aria-label={t('settings.appearance.tabBarAlwaysShowLabels')}
            />
          }
        />
      </SettingsSection>

      {/* ── Chat display toggle ────────────────────────────────────── */}
      <SettingsSection title={t('settings.appearance.chatHeading')}>
        <SettingsRow
          htmlFor="switch-assistant-text-mode"
          label={t('settings.appearance.assistantTextMode')}
          description={t('settings.appearance.assistantTextModeDesc')}
          control={
            <SettingsSwitch
              id="switch-assistant-text-mode"
              checked={assistantTextModeEnabled}
              onCheckedChange={toggleAssistantTextMode}
              aria-label={t('settings.appearance.assistantTextMode')}
            />
          }
        />
        <SettingsRow
          htmlFor="switch-hide-agent-insights"
          label={t('settings.appearance.hideAgentInsights')}
          description={t('settings.appearance.hideAgentInsightsDesc')}
          control={
            <SettingsSwitch
              id="switch-hide-agent-insights"
              checked={hideAgentInsights}
              onCheckedChange={toggleHideAgentInsights}
              aria-label={t('settings.appearance.hideAgentInsights')}
            />
          }
        />
      </SettingsSection>

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
