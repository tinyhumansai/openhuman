import type { ReactNode } from 'react';
import { LuLightbulb } from 'react-icons/lu';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import type { ActionableItem, ActionableItemSource, TimeGroup } from '../../types/intelligence';
import Button from '../ui/Button';
import NativeSelect from '../ui/NativeSelect';
import TextField from '../ui/TextField';
import { ActionableCard } from './ActionableCard';

/**
 * Frosted state panel used for this tab's loading / analyzing / empty states.
 *
 * Replaces the former `.glass` CSS class, which was deleted along with the rest
 * of the bespoke component layer in `index.css`. The values here are a faithful
 * port of it — `blur(12px)` (`backdrop-blur-lg`), `rgb(var(--surface) / 0.8)`,
 * a `rgb(var(--line) / 0.6)` hairline, and the light/dark shadow pair. The two
 * shadow alphas were hardcoded in the original rule too; they are not a new
 * themeability hole, but they are the one part of this that will not follow a
 * custom theme.
 *
 * It exists as a component rather than a shared class string because all three
 * call sites also share the medallion + heading + hint structure. `Card` and
 * `EmptyState` from `components/ui` were both considered and neither fits:
 * `Card` has no translucency and imposes its own heading/divider structure,
 * `EmptyState` is bare text with no icon slot.
 */
function GlassStatePanel({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div
      className={cn(
        'backdrop-blur-lg bg-surface/80 border border-line/60',
        'shadow-[0_2px_8px_rgba(0,0,0,0.06)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.4)]',
        'rounded-2xl p-8 text-center animate-fade-up'
      )}>
      <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
        {icon}
      </div>
      {children}
    </div>
  );
}

/** Indeterminate ring used by the loading and analyzing states. */
function PanelSpinner() {
  return (
    <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
  );
}

interface IntelligenceMemoryTabProps {
  handleAnalyzeNow: () => Promise<void>;
  handleComplete: (item: ActionableItem) => Promise<void>;
  handleDismiss: (item: ActionableItem) => void;
  handleSnooze: (item: ActionableItem, duration: number) => Promise<void>;
  isRunning: boolean;
  items: ActionableItem[];
  itemsLoading: boolean;
  searchFilter: string;
  setSearchFilter: (value: string) => void;
  setSourceFilter: (value: ActionableItemSource | 'all') => void;
  sourceFilter: ActionableItemSource | 'all';
  timeGroups: TimeGroup[];
  usingMemoryData: boolean;
}

export default function IntelligenceMemoryTab({
  handleAnalyzeNow,
  handleComplete,
  handleDismiss,
  handleSnooze,
  isRunning,
  items,
  itemsLoading,
  searchFilter,
  setSearchFilter,
  setSourceFilter,
  sourceFilter,
  timeGroups,
  usingMemoryData,
}: IntelligenceMemoryTabProps) {
  const { t } = useT();
  return (
    <>
      <div className="flex items-center gap-3 mb-6 animate-fade-up">
        <div className="flex-1">
          <label htmlFor="actionable-search" className="sr-only">
            {t('memory.searchAria')}
          </label>
          <TextField
            id="actionable-search"
            type="text"
            placeholder={t('memory.search')}
            value={searchFilter}
            onChange={e => setSearchFilter(e.target.value)}
            className="w-full"
          />
        </div>
        <label htmlFor="actionable-source" className="sr-only">
          {t('memory.sourceFilterAria')}
        </label>
        <NativeSelect
          id="actionable-source"
          value={sourceFilter}
          onChange={e => setSourceFilter(e.target.value as ActionableItemSource | 'all')}>
          <option value="all">{t('memory.sourceFilter.all')}</option>
          <option value="email">{t('memory.sourceFilter.email')}</option>
          <option value="calendar">{t('memory.sourceFilter.calendar')}</option>
          <option value="telegram">{t('memory.sourceFilter.telegram')}</option>
          <option value="ai_insight">{t('memory.sourceFilter.aiInsight')}</option>
          <option value="system">{t('memory.sourceFilter.system')}</option>
          <option value="trading">{t('memory.sourceFilter.trading')}</option>
          <option value="security">{t('memory.sourceFilter.security')}</option>
        </NativeSelect>
      </div>

      {itemsLoading && !usingMemoryData ? (
        <GlassStatePanel icon={<PanelSpinner />}>
          <h2 className="text-lg font-semibold text-content mb-2">{t('memory.loading')}</h2>
          <p className="text-content-faint text-sm">{t('memory.fetching')}</p>
        </GlassStatePanel>
      ) : isRunning && items.length === 0 ? (
        <GlassStatePanel icon={<PanelSpinner />}>
          <h2 className="text-lg font-semibold text-content mb-2">{t('memory.analyzing')}</h2>
          <p className="text-content-faint text-sm">{t('memory.analyzingHint')}</p>
        </GlassStatePanel>
      ) : timeGroups.length === 0 ? (
        <GlassStatePanel icon={<LuLightbulb className="w-8 h-8 text-primary-400" />}>
          {searchFilter || sourceFilter !== 'all' ? (
            <>
              <h2 className="text-lg font-semibold text-content mb-2">{t('memory.noMatches')}</h2>
              <p className="text-content-faint text-sm">{t('memory.noMatchesHint')}</p>
            </>
          ) : usingMemoryData ? (
            <>
              <h2 className="text-lg font-semibold text-content mb-2">{t('memory.allCaughtUp')}</h2>
              <p className="text-content-faint text-sm">{t('memory.allCaughtUpHint')}</p>
            </>
          ) : (
            <>
              <h2 className="text-lg font-semibold text-content mb-2">{t('memory.noAnalysis')}</h2>
              <p className="text-content-faint text-sm mb-4">{t('memory.noAnalysisHint')}</p>
              <Button
                variant="primary"
                size="md"
                onClick={() => void handleAnalyzeNow()}
                disabled={isRunning}>
                {t('memory.analyzeNow')}
              </Button>
            </>
          )}
        </GlassStatePanel>
      ) : (
        <div className="space-y-6">
          {isRunning && (
            <div className="flex items-center gap-2 text-xs text-content-faint animate-fade-up">
              <div className="w-3 h-3 border border-line-strong border-t-transparent rounded-full animate-spin" />
              {t('memory.analyzing')}
            </div>
          )}
          {timeGroups.map((group, groupIndex) => (
            <div
              key={group.label}
              className="animate-fade-up"
              style={{ animationDelay: `${groupIndex * 50}ms` }}>
              <div className="flex items-center justify-between mb-3">
                <h2 className="text-sm font-semibold text-content opacity-80">{group.label}</h2>
                <div className="text-xs bg-surface-subtle text-content px-2 py-1 rounded-full">
                  {group.count}
                </div>
              </div>
              <div className="space-y-3">
                {group.items.map((item, itemIndex) => (
                  <div
                    key={item.id}
                    className="animate-fade-up"
                    style={{ animationDelay: `${groupIndex * 50 + itemIndex * 25}ms` }}>
                    <ActionableCard
                      item={item}
                      onComplete={handleComplete}
                      onDismiss={handleDismiss}
                      onSnooze={handleSnooze}
                    />
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
