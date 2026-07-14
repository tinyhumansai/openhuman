import { useCallback, useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import type { TriggerKind } from '../../hooks/useSubconscious';
import { useT } from '../../lib/i18n/I18nContext';
import type { SubconsciousMode } from '../../utils/tauriCommands/heartbeat';
import type {
  SubconsciousInstanceStatus,
  SubconsciousStatus,
} from '../../utils/tauriCommands/subconscious';
import { settingsNavState } from '../settings/modal/settingsOverlay';
import SubconsciousInstanceCard from './SubconsciousInstanceCard';

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
  /** Per-world status rows (falls back to [memory] on an older core). */
  instances?: SubconsciousInstanceStatus[];
  mode: SubconsciousMode;
  intervalMinutes: number;
  triggerTick: (kind?: TriggerKind) => Promise<void>;
  triggering: boolean;
  /** Per-kind in-flight state (two Run buttons must not share one spinner). */
  isTriggering?: (kind: TriggerKind) => boolean;
  settingMode: boolean;
  setMode: (mode: SubconsciousMode) => Promise<void>;
  setIntervalMinutes: (minutes: number) => Promise<void>;
  /** Navigate to the TinyPlace Orchestration tab's Subconscious window. */
  onViewDirectives?: () => void;
}

export default function IntelligenceSubconsciousTab({
  status,
  instances,
  mode,
  intervalMinutes,
  triggerTick,
  triggering,
  isTriggering,
  settingMode,
  setMode,
  setIntervalMinutes,
}: IntelligenceSubconsciousTabProps) {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const isEnabled = mode !== 'off';

  // Derive the per-world rows, tolerating an older core (no `instances`).
  const rows: SubconsciousInstanceStatus[] =
    instances && instances.length > 0
      ? instances
      : status
        ? [{ ...status, instance: status.instance ?? 'memory' }]
        : [];
  const memoryRow = rows.find(r => r.instance === 'memory') ?? status ?? undefined;
  const running = (kind: TriggerKind) => (isTriggering ? isTriggering(kind) : triggering);
  const openProviderSettings = () => navigate('/settings/llm', settingsNavState(location));

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

  const runTick = (kind: TriggerKind) => {
    Promise.resolve(triggerTick(kind)).catch(error => {
      console.debug('[subconscious-ui] run tick:error', {
        kind,
        error: error instanceof Error ? error.message : String(error),
      });
    });
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

      {/* Per-world instance cards */}
      {isEnabled && (
        <div className="space-y-3">
          <SubconsciousInstanceCard
            title={t('subconscious.instance.memory.title')}
            subtitle={t('subconscious.instance.memory.subtitle')}
            status={memoryRow}
            runLabel={t('subconscious.runNow')}
            triggering={running('memory')}
            onRun={() => runTick('memory')}
            onProviderSettings={openProviderSettings}
          />
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
