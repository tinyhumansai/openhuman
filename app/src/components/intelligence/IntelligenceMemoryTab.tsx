import { useT } from '../../lib/i18n/I18nContext';
import type { ActionableItem, ActionableItemSource, TimeGroup } from '../../types/intelligence';
import { ActionableCard } from './ActionableCard';

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
          <input
            id="actionable-search"
            type="text"
            placeholder={t('memory.search')}
            value={searchFilter}
            onChange={e => setSearchFilter(e.target.value)}
            className="w-full px-3 py-2 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50 transition-colors"
          />
        </div>
        <label htmlFor="actionable-source" className="sr-only">
          Filter by source
        </label>
        <select
          id="actionable-source"
          value={sourceFilter}
          onChange={e => setSourceFilter(e.target.value as ActionableItemSource | 'all')}
          className="px-3 py-2 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 focus:outline-none focus:border-primary-500/50 transition-colors">
          <option value="all">{t('memory.sourceFilter.all')}</option>
          <option value="email">{t('memory.sourceFilter.email')}</option>
          <option value="calendar">{t('memory.sourceFilter.calendar')}</option>
          <option value="telegram">{t('memory.sourceFilter.telegram')}</option>
          <option value="ai_insight">{t('memory.sourceFilter.aiInsight')}</option>
          <option value="system">{t('memory.sourceFilter.system')}</option>
          <option value="trading">{t('memory.sourceFilter.trading')}</option>
          <option value="security">{t('memory.sourceFilter.security')}</option>
        </select>
      </div>

      {itemsLoading && !usingMemoryData ? (
        <div className="glass rounded-2xl p-8 text-center animate-fade-up">
          <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
            <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
          </div>
          <h2 className="text-lg font-semibold text-stone-900 mb-2">{t('memory.loading')}</h2>
          <p className="text-stone-400 text-sm">{t('memory.fetching')}</p>
        </div>
      ) : isRunning && items.length === 0 ? (
        <div className="glass rounded-2xl p-8 text-center animate-fade-up">
          <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
            <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
          </div>
          <h2 className="text-lg font-semibold text-stone-900 mb-2">{t('memory.analyzing')}</h2>
          <p className="text-stone-400 text-sm">{t('memory.analyzingHint')}</p>
        </div>
      ) : timeGroups.length === 0 ? (
        <div className="glass rounded-2xl p-8 text-center animate-fade-up">
          <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
            <svg
              className="w-8 h-8 text-primary-400"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
              />
            </svg>
          </div>
          {searchFilter || sourceFilter !== 'all' ? (
            <>
              <h2 className="text-lg font-semibold text-stone-900 mb-2">{t('memory.noMatches')}</h2>
              <p className="text-stone-400 text-sm">{t('memory.noMatchesHint')}</p>
            </>
          ) : usingMemoryData ? (
            <>
              <h2 className="text-lg font-semibold text-stone-900 mb-2">
                {t('memory.allCaughtUp')}
              </h2>
              <p className="text-stone-400 text-sm">{t('memory.allCaughtUpHint')}</p>
            </>
          ) : (
            <>
              <h2 className="text-lg font-semibold text-stone-900 mb-2">
                {t('memory.noAnalysis')}
              </h2>
              <p className="text-stone-400 text-sm mb-4">{t('memory.noAnalysisHint')}</p>
              <button
                onClick={() => void handleAnalyzeNow()}
                disabled={isRunning}
                className="px-4 py-2 bg-primary-500 hover:bg-primary-600 disabled:opacity-40 text-white text-sm rounded-lg transition-colors">
                {t('memory.analyzeNow')}
              </button>
            </>
          )}
        </div>
      ) : (
        <div className="space-y-6">
          {isRunning && (
            <div className="flex items-center gap-2 text-xs text-stone-400 animate-fade-up">
              <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
              {t('memory.analyzing')}
            </div>
          )}
          {timeGroups.map((group, groupIndex) => (
            <div
              key={group.label}
              className="animate-fade-up"
              style={{ animationDelay: `${groupIndex * 50}ms` }}>
              <div className="flex items-center justify-between mb-3">
                <h2 className="text-sm font-semibold text-stone-900 opacity-80">{group.label}</h2>
                <div className="text-xs bg-stone-100 text-stone-900 px-2 py-1 rounded-full">
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
