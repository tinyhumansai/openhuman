import { useCallback, useEffect, useMemo, useState } from 'react';
import { useDispatch, useSelector } from 'react-redux';

import { ActionableCard } from '../components/intelligence/ActionableCard';
import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import { ToastContainer } from '../components/intelligence/Toast';
import { filterItems, getItemStats, groupItemsByTime } from '../components/intelligence/utils';
import {
  useActionableItems,
  useSnoozeActionableItem,
  useUpdateActionableItem,
} from '../hooks/useIntelligenceApiFallback';
import { useIntelligenceSocket, useIntelligenceSocketManager } from '../hooks/useIntelligenceSocket';
import { useIntelligenceStats } from '../hooks/useIntelligenceStats';
import type { RootState } from '../store';
import {
  setSourceFilter,
  setSearchFilter,
} from '../store/intelligenceSlice';
import type {
  ActionableItem,
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

  // API hooks
  const {
    data: apiItems,
    loading: itemsLoading,
    error: itemsError,
    refetch: refetchItems,
  } = useActionableItems();

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

  // Use API data or fallback to empty array
  const items = apiItems || [];

  // Initialize socket connection
  useEffect(() => {
    if (!socketConnected) {
      socketManager.connect();
    }
  }, [socketConnected, socketManager]);

  // Handle API errors with toast notifications
  useEffect(() => {
    if (itemsError) {
      addToast({
        type: 'error',
        title: 'Failed to load items',
        message: typeof itemsError === 'string' ? itemsError : 'Unable to fetch actionable items',
      });
    }
  }, [itemsError]);

  // Filter and group items
  const filteredItems = useMemo(() => {
    const activeItems = items.filter(item => item.status === 'active');
    return filterItems(activeItems, {
      source: filters.source,
      priority: filters.priority,
      searchTerm: filters.search,
    });
  }, [items, filters.source, filters.priority, filters.search]);

  const timeGroups = useMemo(() => {
    return groupItemsByTime(filteredItems);
  }, [filteredItems]);

  const stats = useMemo(() => {
    return getItemStats(filteredItems);
  }, [filteredItems]);

  // Toast utilities
  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);

  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);

  // Item action handlers with real backend integration
  const handleUpdateItemStatus = useCallback(async (itemId: string, status: ActionableItemStatus) => {
    try {
      await updateItemStatus({ itemId, status });

      // Success toast
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

      addToast({
        type: 'success',
        title: 'Status Updated',
        message,
      });
    } catch (error) {
      console.error('Failed to update item status:', error);
      addToast({
        type: 'error',
        title: 'Update Failed',
        message: error instanceof Error ? error.message : 'Failed to update item status',
      });
    }
  }, [updateItemStatus]);

  const handleComplete = useCallback(async (item: ActionableItem) => {
    await handleUpdateItemStatus(item.id, 'completed');
  }, [handleUpdateItemStatus]);

  const handleDismiss = useCallback(
    (item: ActionableItem) => {
      // Always show confirmation modal for ALL dismiss actions
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

            // Add undo action to toast
            addToast({
              type: 'info',
              title: 'Dismissed',
              message: item.title.length > 40 ? `${item.title.substring(0, 40)}...` : item.title,
              action: {
                label: 'Undo',
                handler: () => handleUpdateItemStatus(item.id, 'active'),
              },
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

  // Combined AI and socket status indicator
  const systemStatus = socketConnected && aiStatus === 'ready' ? 'ready' :
                      itemsLoading ? 'loading' :
                      !socketConnected ? 'disconnected' : aiStatus;

  const systemStatusLabel =
    systemStatus === 'ready' ? 'System Ready' :
    systemStatus === 'loading' ? 'Loading...' :
    systemStatus === 'disconnected' ? 'Connecting...' :
    systemStatus === 'initializing' ? 'Initializing...' :
    systemStatus === 'error' ? 'System Error' : 'System Idle';

  const systemStatusDot =
    systemStatus === 'ready' ? 'bg-sage-400' :
    systemStatus === 'loading' ? 'bg-amber-400 animate-pulse' :
    systemStatus === 'disconnected' ? 'bg-amber-400 animate-pulse' :
    systemStatus === 'initializing' ? 'bg-amber-400 animate-pulse' :
    systemStatus === 'error' ? 'bg-coral-400' : 'bg-stone-600';

  return (
    <div className="min-h-full relative">
      <div className="relative z-10 min-h-full flex flex-col">
        <div className="flex-1 p-6">
          <div className="max-w-2xl mx-auto">
            {/* Header */}
            <div className="flex items-center justify-between mb-6">
              <div className="flex items-center gap-3">
                <h1 className="text-xl font-bold text-white">Intelligence</h1>
                {stats.total > 0 && (
                  <div className="text-xs bg-white/10 text-white px-2 py-1 rounded-full">
                    {stats.total}
                  </div>
                )}
              </div>
              <div className="flex items-center gap-2">
                <div className={`w-2 h-2 rounded-full ${systemStatusDot}`} />
                <span className="text-xs text-stone-400">{systemStatusLabel}</span>
              </div>
            </div>

            {/* Filters */}
            <div className="flex items-center gap-3 mb-6 animate-fade-up">
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
                onChange={e => dispatch(setSourceFilter(e.target.value as any))}
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
            {itemsLoading ? (
              /* Loading State */
              <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
                  <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin"></div>
                </div>
                <h2 className="text-lg font-semibold text-white mb-2">Loading Intelligence...</h2>
                <p className="text-stone-400 text-sm">Fetching your actionable items</p>
              </div>
            ) : itemsError ? (
              /* Error State */
              <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-coral-500/10">
                  <svg
                    className="w-8 h-8 text-coral-400"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                    />
                  </svg>
                </div>
                <h2 className="text-lg font-semibold text-white mb-2">Unable to load items</h2>
                <p className="text-stone-400 text-sm mb-4">
                  {typeof itemsError === 'string' ? itemsError : 'Something went wrong'}
                </p>
                <button
                  onClick={() => refetchItems()}
                  className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white text-sm rounded-lg transition-colors"
                >
                  Try Again
                </button>
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
                      d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                    />
                  </svg>
                </div>
                <h2 className="text-lg font-semibold text-white mb-2">All caught up!</h2>
                <p className="text-stone-400 text-sm">
                  {filters.search || filters.source !== 'all'
                    ? 'No items match your current filters.'
                    : 'No actionable items at the moment. Great work!'}
                </p>
              </div>
            ) : (
              /* Time Groups */
              <div className="space-y-6">
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
