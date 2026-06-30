import { useCallback, useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import type { SubconsciousMode } from '../../utils/tauriCommands/heartbeat';
import type { SubconsciousStatus } from '../../utils/tauriCommands/subconscious';
import { settingsNavState } from '../settings/modal/settingsOverlay';
import Button from '../ui/Button';

interface ModeOption {
  id: SubconsciousMode;
  titleKey: string;
  descKey: string;
}

const MODE_OPTIONS: ModeOption[] = [
  { id: 'off', titleKey: 'subconscious.mode.off.title', descKey: 'subconscious.mode.off.desc' },
  {
    id: 'simple',
    titleKey: 'subconscious.mode.simple.title',
    descKey: 'subconscious.mode.simple.desc',
  },
  {
    id: 'aggressive',
    titleKey: 'subconscious.mode.aggressive.title',
    descKey: 'subconscious.mode.aggressive.desc',
  },
];

const INTERVAL_STOPS = [5, 10, 15, 30, 60, 120, 360, 720, 1440];

function formatMinutes(minutes: number, t: (key: string) => string): string {
  if (minutes < 60) return t('subconscious.interval.minutes').replace('{n}', String(minutes));
  const hours = minutes / 60;
  if (hours === 1) return t('subconscious.interval.oneHour');
  if (hours === 24) return t('subconscious.interval.oneDay');
  return t('subconscious.interval.hours').replace('{n}', String(hours));
}

function minutesToSlider(minutes: number): number {
  const idx = INTERVAL_STOPS.indexOf(minutes);
  return idx >= 0 ? idx : 0;
}

function sliderToMinutes(value: number): number {
  return INTERVAL_STOPS[value] ?? 30;
}

interface IntelligenceSubconsciousTabProps {
  status: SubconsciousStatus | null;
  mode: SubconsciousMode;
  intervalMinutes: number;
  triggerTick: () => Promise<void>;
  triggering: boolean;
  settingMode: boolean;
  setMode: (mode: SubconsciousMode) => Promise<void>;
  setIntervalMinutes: (minutes: number) => Promise<void>;
}

export default function IntelligenceSubconsciousTab({
  status,
  mode,
  intervalMinutes,
  triggerTick,
  triggering,
  settingMode,
  setMode,
  setIntervalMinutes,
}: IntelligenceSubconsciousTabProps) {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const providerUnavailable = status?.provider_available === false;
  const providerUnavailableReason = providerUnavailable
    ? (status?.provider_unavailable_reason ?? t('subconscious.providerUnavailableTitle'))
    : null;
  const isEnabled = mode !== 'off';

  const [localSlider, setLocalSlider] = useState(() => minutesToSlider(intervalMinutes));

  // Keep the local slider in sync when the prop changes from outside (e.g. after a refresh).
  useEffect(() => {
    setLocalSlider(minutesToSlider(intervalMinutes));
  }, [intervalMinutes]);

  const handleSliderChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const val = Number(e.target.value);
    setLocalSlider(val);
  }, []);

  const handleSliderCommit = useCallback(() => {
    const minutes = sliderToMinutes(localSlider);
    if (minutes !== intervalMinutes) {
      void setIntervalMinutes(minutes);
    }
  }, [localSlider, intervalMinutes, setIntervalMinutes]);

  const handleRunTick = async () => {
    try {
      await triggerTick();
    } catch (error) {
      console.debug('[subconscious-ui] run tick:error', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  return (
    <div className="space-y-5 animate-fade-up">
      {/* Mode selector */}
      <div>
        <h3 className="text-sm font-semibold text-content mb-2">{t('subconscious.mode.label')}</h3>
        <div className="grid grid-cols-3 gap-2">
          {MODE_OPTIONS.map(opt => (
            <button
              key={opt.id}
              type="button"
              disabled={settingMode}
              onClick={() => void setMode(opt.id)}
              className={`flex flex-col items-center text-center rounded-lg border p-3 transition ${
                mode === opt.id
                  ? 'border-primary-500 bg-primary-50 dark:bg-primary-500/10'
                  : 'border-line hover:border-primary-300 dark:hover:border-primary-500/40'
              } ${settingMode ? 'opacity-60 cursor-wait' : ''}`}>
              <span
                className={`inline-block w-3 h-3 rounded-full border-2 mb-1.5 ${
                  mode === opt.id
                    ? 'bg-primary-500 border-primary-500'
                    : 'border-line-strong dark:border-neutral-600'
                }`}
              />
              <span className="text-sm font-medium text-content">{t(opt.titleKey)}</span>
              <p className="mt-1 text-[11px] leading-tight text-content-muted">{t(opt.descKey)}</p>
            </button>
          ))}
        </div>
        {mode === 'aggressive' && (
          <p className="mt-2 text-xs text-amber-600 dark:text-amber-400">
            {t('subconscious.mode.aggressiveWarning')}
          </p>
        )}
      </div>

      {/* Frequency slider */}
      {isEnabled && (
        <div>
          <div className="flex items-center justify-between mb-1.5">
            <label className="text-xs font-medium text-content-secondary">
              {t('subconscious.interval.label')}
            </label>
            <span className="text-xs text-content-muted">
              {formatMinutes(sliderToMinutes(localSlider), t)}
            </span>
          </div>
          <input
            type="range"
            min={0}
            max={INTERVAL_STOPS.length - 1}
            step={1}
            value={localSlider}
            onChange={handleSliderChange}
            onMouseUp={handleSliderCommit}
            onTouchEnd={handleSliderCommit}
            className="w-full h-1.5 rounded-full appearance-none cursor-pointer bg-surface-strong accent-primary-500"
          />
          <div className="flex justify-between mt-1 text-[10px] text-content-faint">
            <span>5m</span>
            <span>1h</span>
            <span>24h</span>
          </div>
        </div>
      )}

      {/* Status bar + Run Now */}
      {isEnabled && (
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-xs text-content-faint">
            {status && (
              <>
                <span>
                  {status.total_ticks} {t('subconscious.ticks')}
                </span>
                {status.last_tick_at && (
                  <>
                    <span className="text-content-faint dark:text-neutral-600">|</span>
                    <span>
                      {t('subconscious.last')}:{' '}
                      {new Date(status.last_tick_at * 1000).toLocaleTimeString()}
                    </span>
                  </>
                )}
                {status.consecutive_failures > 0 && (
                  <>
                    <span className="text-content-faint dark:text-neutral-600">|</span>
                    <span className="text-coral-500">
                      {status.consecutive_failures} {t('subconscious.failed')}
                    </span>
                  </>
                )}
              </>
            )}
          </div>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void handleRunTick()}
            disabled={triggering || providerUnavailable}
            title={providerUnavailable ? t('subconscious.providerUnavailableTitle') : undefined}>
            {triggering ? (
              <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
            ) : (
              <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 10V3L4 14h7v7l9-11h-7z"
                />
              </svg>
            )}
            {t('subconscious.runNow')}
          </Button>
        </div>
      )}

      {isEnabled && providerUnavailable && (
        <div className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm font-medium text-amber-800 dark:text-amber-200">
                {t('subconscious.providerUnavailableTitle')}
              </p>
              <p className="mt-1 text-xs text-amber-700 dark:text-amber-300 break-words">
                {providerUnavailableReason}
              </p>
            </div>
            <button
              type="button"
              onClick={() => navigate('/settings/llm', settingsNavState(location))}
              className="flex-shrink-0 rounded-md bg-amber-600 px-2.5 py-1.5 text-xs font-medium text-content-inverted hover:bg-amber-700 transition-colors">
              {t('subconscious.providerSettings')}
            </button>
          </div>
        </div>
      )}

      {isEnabled && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800/50 p-4">
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            {t('subconscious.scratchpadInfo')}
          </p>
        </div>
      )}
    </div>
  );
}
