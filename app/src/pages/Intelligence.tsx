import { useCallback, useEffect, useMemo, useState } from 'react';

import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import IntelligenceDreamsTab from '../components/intelligence/IntelligenceDreamsTab';
import IntelligenceMemoryTab from '../components/intelligence/IntelligenceMemoryTab';
import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import { ToastContainer } from '../components/intelligence/Toast';
import { filterItems, getItemStats, groupItemsByTime } from '../components/intelligence/utils';
import PillTabBar from '../components/PillTabBar';
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
import { useScreenIntelligenceItems } from '../hooks/useScreenIntelligenceItems';
import { useSubconscious } from '../hooks/useSubconscious';
import type {
  ActionableItem,
  ActionableItemSource,
  ActionableItemStatus,
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../types/intelligence';

type IntelligenceTab = 'memory' | 'subconscious' | 'dreams';

export default function Intelligence() {
  const { aiStatus } = useIntelligenceStats();

  const [activeTab, setActiveTab] = useState<IntelligenceTab>('memory');
  const [sourceFilter, setSourceFilter] = useState<ActionableItemSource | 'all'>('all');
  const [priorityFilter] = useState<'critical' | 'important' | 'normal' | 'all'>('all');
  const [searchFilter, setSearchFilter] = useState('');

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

  // Subconscious engine data
  const {
    tasks: subconsciousTasks,
    escalations,
    logEntries,
    status: subconsciousEngineStatus,
    loading: subconsciousLoading,
    triggering: subconsciousTriggering,
    triggerTick,
    addTask: addSubconsciousTask,
    removeTask: removeSubconsciousTask,
    toggleTask: toggleSubconsciousTask,
    approveEscalation,
    dismissEscalation,
  } = useSubconscious();
  const [newTaskTitle, setNewTaskTitle] = useState('');
  const [expandedLogIds, setExpandedLogIds] = useState<Set<string>>(new Set());

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

  const { items: screenIntelligenceItems, loading: screenIntelligenceLoading } =
    useScreenIntelligenceItems();

  const usingMemoryData = consciousItems.length > 0 || screenIntelligenceItems.length > 0;
  const items: ActionableItem[] = useMemo(
    () => [...consciousItems, ...screenIntelligenceItems],
    [consciousItems, screenIntelligenceItems]
  );

  const itemsLoading = consciousLoading || screenIntelligenceLoading;

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
      source: sourceFilter,
      priority: priorityFilter,
      searchTerm: searchFilter,
    });
  }, [items, priorityFilter, searchFilter, sourceFilter]);

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

  const tabs: { id: IntelligenceTab; label: string; comingSoon?: boolean }[] = [
    { id: 'memory', label: 'Memory' },
    { id: 'subconscious', label: 'Subconscious' },
    { id: 'dreams', label: 'Dreams', comingSoon: true },
  ];

  return (
    <div className="min-h-full p-4 pt-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <PillTabBar
          items={tabs.map(tab => ({ label: tab.label, value: tab.id }))}
          selected={activeTab}
          onChange={setActiveTab}
          activeClassName="border-primary-600 bg-primary-600 text-white"
          renderItem={(item, active) => {
            const tab = tabs.find(entry => entry.id === item.value);
            return (
              <span className="inline-flex items-center gap-1.5">
                <span>{item.label}</span>
                {tab?.comingSoon && (
                  <span
                    className={`rounded-full border px-1.5 py-0.5 text-[10px] ${
                      active
                        ? 'border-white/30 bg-white/15 text-white'
                        : 'border-stone-200 bg-stone-50 text-stone-500'
                    }`}>
                    Soon
                  </span>
                )}
              </span>
            );
          }}
        />

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <div>
            {/* Header */}
            <div className="flex items-center justify-between mb-6">
              <div className="flex items-center gap-3">
                <h1 className="text-xl font-bold text-stone-900">Intelligence</h1>
                {activeTab === 'memory' && stats.total > 0 && (
                  <div className="text-xs bg-stone-100 text-stone-900 px-2 py-1 rounded-full">
                    {stats.total}
                  </div>
                )}
              </div>
              <div className="flex items-center gap-3">
                {activeTab === 'memory' && (
                  <div className="flex items-center gap-2">
                    <div className={`w-2 h-2 rounded-full ${systemStatusDot}`} />
                    <span className="text-xs text-stone-400">{systemStatusLabel}</span>
                  </div>
                )}
                {activeTab === 'memory' && (
                  <button
                    onClick={usingMemoryData ? refreshConscious : handleAnalyzeNow}
                    disabled={isRunning || itemsLoading}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-stone-50 hover:bg-stone-100 disabled:opacity-40 disabled:cursor-not-allowed border border-stone-200 rounded-lg text-stone-600 transition-colors">
                    {isRunning || itemsLoading ? (
                      <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
                    ) : (
                      <svg
                        className="w-3 h-3"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
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
                )}
              </div>
            </div>

            {/* Tab content */}
            {activeTab === 'memory' && (
              <IntelligenceMemoryTab
                handleAnalyzeNow={handleAnalyzeNow}
                handleComplete={handleComplete}
                handleDismiss={handleDismiss}
                handleSnooze={handleSnooze}
                isRunning={isRunning}
                items={items}
                itemsLoading={itemsLoading}
                searchFilter={searchFilter}
                setSearchFilter={setSearchFilter}
                setSourceFilter={setSourceFilter}
                sourceFilter={sourceFilter}
                timeGroups={timeGroups}
                usingMemoryData={usingMemoryData}
              />
            )}

            {activeTab === 'subconscious' && (
              <IntelligenceSubconsciousTab
                addSubconsciousTask={addSubconsciousTask}
                approveEscalation={approveEscalation}
                dismissEscalation={dismissEscalation}
                escalations={escalations}
                expandedLogIds={expandedLogIds}
                loading={subconsciousLoading}
                logEntries={logEntries}
                newTaskTitle={newTaskTitle}
                removeSubconsciousTask={removeSubconsciousTask}
                setExpandedLogIds={setExpandedLogIds}
                setNewTaskTitle={setNewTaskTitle}
                status={subconsciousEngineStatus}
                tasks={subconsciousTasks}
                toggleSubconsciousTask={toggleSubconsciousTask}
                triggerTick={triggerTick}
                triggering={subconsciousTriggering}
              />
            )}

            {activeTab === 'dreams' && <IntelligenceDreamsTab />}
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
