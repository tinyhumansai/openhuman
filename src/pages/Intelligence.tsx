import { useCallback, useEffect, useMemo, useState } from 'react';
import { useDispatch, useSelector } from 'react-redux';

import { ActionableCard } from '../components/intelligence/ActionableCard';
import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import { MemoryWorkspace } from '../components/intelligence/MemoryWorkspace';
import { ToastContainer } from '../components/intelligence/Toast';
import { filterItems, getItemStats, groupItemsByTime } from '../components/intelligence/utils';
import { useConsciousItems } from '../hooks/useConsciousItems';
import {
  useSnoozeActionableItem,
  useUpdateActionableItem,
} from '../hooks/useIntelligenceApiFallback';
import {
  useIntelligenceSocket,
  useIntelligenceSocketManager,
} from '../hooks/useIntelligenceSocket';
import { useIntelligenceStats } from '../hooks/useIntelligenceStats';
import type { RootState } from '../store';
import { setSearchFilter, setSourceFilter } from '../store/intelligenceSlice';
import type {
  ActionableItem,
  ActionableItemSource,
  ActionableItemStatus,
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../types/intelligence';

export default function Intelligence() {
  const dispatch = useDispatch();
  const { aiStatus } = useIntelligenceStats();

  // Redux state
  const intelligenceState = useSelector((state: RootState) => state.intelligence);
  const { filters } = intelligenceState;

  // Conscious memory items (real data from the background analysis loop)
  const {
    items: consciousItems,
    loading: consciousLoading,
    isRunning,
    refresh: refreshConscious,
    triggerAnalysis,
  } = useConsciousItems();

  const { mutateAsync: updateItemStatus } = useUpdateActionableItem();
  const { mutateAsync: snoozeItem } = useSnoozeActionableItem();

  // Socket integration
  const socketManager = useIntelligenceSocketManager();
  const { isConnected: socketConnected } = useIntelligenceSocket();

  // Local state for UI
  const [toasts, setToasts] = useState<ToastNotification[]>([]);
  const [confirmationModal, setConfirmationModal] = useState<ConfirmationModalType>({
    isOpen: false,
    title: '',
    message: '',
    onConfirm: () => {},
    onCancel: () => {},
  });

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);

  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);

  const usingMemoryData = consciousItems.length > 0;
  const items: ActionableItem[] = useMemo(() => consciousItems, [consciousItems]);

  const itemsLoading = consciousLoading;

  // Initialize socket connection
  useEffect(() => {
    if (!socketConnected) {
      socketManager.connect();
    }
  }, [socketConnected, socketManager]);

  // Filter and group items
  const filteredItems = useMemo(() => {
    const activeItems = items.filter(item => item.status === 'active');
    return filterItems(activeItems, {
      source: filters.source,
      priority: filters.priority,
      searchTerm: filters.search,
    });
  }, [items, filters.source, filters.priority, filters.search]);

  const timeGroups = useMemo(() => groupItemsByTime(filteredItems), [filteredItems]);
  const stats = useMemo(() => getItemStats(filteredItems), [filteredItems]);

  // Item action handlers
  const handleUpdateItemStatus = useCallback(
    async (itemId: string, status: ActionableItemStatus) => {
      try {
        await updateItemStatus({ itemId, status });

        let message = '';
        switch (status) {
          case 'completed':
            message = 'Task marked as completed';
            break;
          case 'dismissed':
            message = 'Task dismissed';
            break;
          case 'active':
            message = 'Task reactivated';
            break;
          default:
            message = 'Status updated';
        }

        addToast({ type: 'success', title: 'Status Updated', message });
      } catch (error) {
        console.error('Failed to update item status:', error);
        addToast({
          type: 'error',
          title: 'Update Failed',
          message: error instanceof Error ? error.message : 'Failed to update item status',
        });
      }
    },
    [updateItemStatus, addToast]
  );

  const handleComplete = useCallback(
    async (item: ActionableItem) => {
      await handleUpdateItemStatus(item.id, 'completed');
    },
    [handleUpdateItemStatus]
  );

  const handleDismiss = useCallback(
    (item: ActionableItem) => {
      setConfirmationModal({
        isOpen: true,
        title: 'Dismiss item?',
        message: `Are you sure you want to dismiss "${item.title}"?`,
        confirmText: 'Dismiss',
        cancelText: 'Cancel',
        destructive: item.priority === 'critical',
        showDontShowAgain: !item.requiresConfirmation,
        onConfirm: async () => {
          try {
            await handleUpdateItemStatus(item.id, 'dismissed');
            addToast({
              type: 'info',
              title: 'Dismissed',
              message: item.title.length > 40 ? `${item.title.substring(0, 40)}...` : item.title,
              action: { label: 'Undo', handler: () => handleUpdateItemStatus(item.id, 'active') },
            });
          } catch (error) {
            console.error('Failed to dismiss item:', error);
          }
        },
        onCancel: () => {},
      });
    },
    [handleUpdateItemStatus, addToast]
  );

  const handleSnooze = useCallback(
    async (item: ActionableItem, duration: number) => {
      try {
        const snoozeUntil = new Date(Date.now() + duration);
        await snoozeItem({ itemId: item.id, snoozeUntil });
        const hours = Math.round(duration / (1000 * 60 * 60));
        addToast({
          type: 'info',
          title: 'Snoozed',
          message: `Reminded in ${hours === 1 ? '1 hour' : `${hours} hours`}`,
        });
      } catch (error) {
        console.error('Failed to snooze item:', error);
        addToast({
          type: 'error',
          title: 'Snooze Failed',
          message: 'Failed to snooze item. Please try again.',
        });
      }
    },
    [snoozeItem, addToast]
  );

  const handleAnalyzeNow = useCallback(async () => {
    await triggerAnalysis();
    addToast({
      type: 'info',
      title: 'Analysis Started',
      message: 'Analyzing your connected skills for actionable items…',
    });
  }, [triggerAnalysis, addToast]);

  // System status
  const systemStatus = isRunning
    ? 'loading'
    : socketConnected && aiStatus === 'ready'
      ? 'ready'
      : itemsLoading
        ? 'loading'
        : !socketConnected
          ? 'disconnected'
          : aiStatus;

  const systemStatusLabel = isRunning
    ? 'Analyzing…'
    : systemStatus === 'ready'
      ? 'System Ready'
      : systemStatus === 'loading'
        ? 'Loading…'
        : systemStatus === 'disconnected'
          ? 'Connecting…'
          : systemStatus === 'initializing'
            ? 'Initializing…'
            : systemStatus === 'error'
              ? 'System Error'
              : 'System Idle';

  const systemStatusDot =
    isRunning || systemStatus === 'loading'
      ? 'bg-amber-400 animate-pulse'
      : systemStatus === 'ready'
        ? 'bg-sage-400'
        : systemStatus === 'disconnected' || systemStatus === 'initializing'
          ? 'bg-amber-400 animate-pulse'
          : systemStatus === 'error'
            ? 'bg-coral-400'
            : 'bg-stone-600';

  return (
    <div className="min-h-full relative">
      <div className="relative z-10 min-h-full flex flex-col">
        <div className="flex-1 p-6">
          <div className="max-w-6xl mx-auto">
            {/* Header */}
            <div className="flex items-center justify-between mb-6">
              <div className="flex items-center gap-3">
                <h1 className="text-xl font-bold text-white">Intelligence</h1>
                {stats.total > 0 && (
                  <div className="text-xs bg-white/10 text-white px-2 py-1 rounded-full">
                    {stats.total}
                  </div>
                )}
                {usingMemoryData && (
                  <div className="text-xs bg-primary-500/20 text-primary-300 px-2 py-1 rounded-full border border-primary-500/20">
                    Memory
                  </div>
                )}
              </div>
              <div className="flex items-center gap-3">
                <div className="flex items-center gap-2">
                  <div className={`w-2 h-2 rounded-full ${systemStatusDot}`} />
                  <span className="text-xs text-stone-400">{systemStatusLabel}</span>
                </div>
                {/* Analyze Now / Refresh button */}
                <button
                  onClick={usingMemoryData ? refreshConscious : handleAnalyzeNow}
                  disabled={isRunning || itemsLoading}
                  className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-white/5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed border border-white/10 rounded-lg text-stone-300 transition-colors">
                  {isRunning || itemsLoading ? (
                    <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
                  ) : (
                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                      />
                    </svg>
                  )}
                  {usingMemoryData ? 'Refresh' : 'Analyze Now'}
                </button>
              </div>
            </div>

            <MemoryWorkspace onToast={addToast} />

            {/* Filters */}
            <div className="flex items-center gap-3 mt-6 mb-6 animate-fade-up">
              <div className="flex-1">
                <input
                  type="text"
                  placeholder="Search actionable items..."
                  value={filters.search}
                  onChange={e => dispatch(setSearchFilter(e.target.value))}
                  className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white placeholder-stone-500 focus:outline-none focus:border-primary-500/50 transition-colors"
                />
              </div>
              <select
                value={filters.source}
                onChange={e =>
                  dispatch(setSourceFilter(e.target.value as ActionableItemSource | 'all'))
                }
                className="px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white focus:outline-none focus:border-primary-500/50 transition-colors">
                <option value="all">All Sources</option>
                <option value="email">Email</option>
                <option value="calendar">Calendar</option>
                <option value="telegram">Telegram</option>
                <option value="ai_insight">AI Insights</option>
                <option value="system">System</option>
                <option value="trading">Trading</option>
                <option value="security">Security</option>
              </select>
            </div>

            {/* Content */}
            {itemsLoading && !usingMemoryData ? (
              /* Loading State */
              <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
                  <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
                </div>
                <h2 className="text-lg font-semibold text-white mb-2">Loading Intelligence...</h2>
                <p className="text-stone-400 text-sm">Fetching your actionable items</p>
              </div>
            ) : isRunning && items.length === 0 ? (
              /* Analyzing State (no items yet) */
              <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
                  <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
                </div>
                <h2 className="text-lg font-semibold text-white mb-2">Analyzing your data…</h2>
                <p className="text-stone-400 text-sm">
                  The conscious loop is reviewing your connected skills
                </p>
              </div>
            ) : timeGroups.length === 0 ? (
              /* Empty State */
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
                {filters.search || filters.source !== 'all' ? (
                  <>
                    <h2 className="text-lg font-semibold text-white mb-2">No matches</h2>
                    <p className="text-stone-400 text-sm">No items match your current filters.</p>
                  </>
                ) : usingMemoryData ? (
                  <>
                    <h2 className="text-lg font-semibold text-white mb-2">All caught up!</h2>
                    <p className="text-stone-400 text-sm">No actionable items at the moment.</p>
                  </>
                ) : (
                  <>
                    <h2 className="text-lg font-semibold text-white mb-2">No analysis yet</h2>
                    <p className="text-stone-400 text-sm mb-4">
                      Run an analysis to extract actionable items from your connected skills.
                    </p>
                    <button
                      onClick={handleAnalyzeNow}
                      disabled={isRunning}
                      className="px-4 py-2 bg-primary-500 hover:bg-primary-600 disabled:opacity-40 text-white text-sm rounded-lg transition-colors">
                      Analyze Now
                    </button>
                  </>
                )}
              </div>
            ) : (
              /* Time Groups */
              <div className="space-y-6">
                {/* Inline analyzing indicator when refreshing with existing items */}
                {isRunning && (
                  <div className="flex items-center gap-2 text-xs text-stone-400 animate-fade-up">
                    <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
                    Analyzing your data…
                  </div>
                )}
                {timeGroups.map((group, groupIndex) => (
                  <div
                    key={group.label}
                    className="animate-fade-up"
                    style={{ animationDelay: `${groupIndex * 50}ms` }}>
                    {/* Group Header */}
                    <div className="flex items-center justify-between mb-3">
                      <h2 className="text-sm font-semibold text-white opacity-80">{group.label}</h2>
                      <div className="text-xs bg-white/10 text-white px-2 py-1 rounded-full">
                        {group.count}
                      </div>
                    </div>

                    {/* Items */}
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
          </div>
        </div>
      </div>

      {/* Toast notifications */}
      <ToastContainer notifications={toasts} onRemove={removeToast} />

      {/* Confirmation modal */}
      <ConfirmationModal
        modal={confirmationModal}
        onClose={() => setConfirmationModal(prev => ({ ...prev, isOpen: false }))}
      />
    </div>
  );
}
