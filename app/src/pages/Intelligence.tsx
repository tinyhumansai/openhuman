import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { ActionableCard } from '../components/intelligence/ActionableCard';
import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
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

const SKILL_KEYWORDS =
  /\bskill\b|\boauth\b|\bnotion\b|\bgmail\b|\bintegration\b|\bdisconnect|\breconnect|\bre-?auth/i;

function isSkillRelated(title: string, description: string): boolean {
  return SKILL_KEYWORDS.test(title) || SKILL_KEYWORDS.test(description);
}

export default function Intelligence() {
  const navigate = useNavigate();
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
      <div className="max-w-2xl mx-auto">
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

            {/* Tabs */}
            <div className="flex border-b border-stone-200 mb-6">
              {tabs.map(tab => (
                <button
                  key={tab.id}
                  onClick={() => setActiveTab(tab.id)}
                  className={`relative px-4 py-2.5 text-sm font-medium transition-colors ${
                    activeTab === tab.id
                      ? 'text-primary-400 border-b-2 border-primary-400'
                      : 'text-stone-400 hover:text-stone-700'
                  }`}>
                  {tab.label}
                  {tab.comingSoon && (
                    <span className="ml-1.5 text-[10px] px-1.5 py-0.5 rounded-full bg-stone-50 text-stone-500 border border-stone-200">
                      Soon
                    </span>
                  )}
                </button>
              ))}
            </div>

            {/* Tab content */}
            {activeTab === 'memory' && (
              <>
                {/* Filters */}
                <div className="flex items-center gap-3 mb-6 animate-fade-up">
                  <div className="flex-1">
                    <input
                      type="text"
                      placeholder="Search actionable items..."
                      value={searchFilter}
                      onChange={e => setSearchFilter(e.target.value)}
                      className="w-full px-3 py-2 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50 transition-colors"
                    />
                  </div>
                  <select
                    value={sourceFilter}
                    onChange={e => setSourceFilter(e.target.value as ActionableItemSource | 'all')}
                    className="px-3 py-2 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 focus:outline-none focus:border-primary-500/50 transition-colors">
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
                  <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                    <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
                      <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
                    </div>
                    <h2 className="text-lg font-semibold text-stone-900 mb-2">
                      Loading Intelligence...
                    </h2>
                    <p className="text-stone-400 text-sm">Fetching your actionable items</p>
                  </div>
                ) : isRunning && items.length === 0 ? (
                  <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                    <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-primary-500/10">
                      <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin" />
                    </div>
                    <h2 className="text-lg font-semibold text-stone-900 mb-2">
                      Analyzing your data…
                    </h2>
                    <p className="text-stone-400 text-sm">
                      The conscious loop is reviewing your connected skills
                    </p>
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
                        <h2 className="text-lg font-semibold text-stone-900 mb-2">No matches</h2>
                        <p className="text-stone-400 text-sm">
                          No items match your current filters.
                        </p>
                      </>
                    ) : usingMemoryData ? (
                      <>
                        <h2 className="text-lg font-semibold text-stone-900 mb-2">
                          All caught up!
                        </h2>
                        <p className="text-stone-400 text-sm">No actionable items at the moment.</p>
                      </>
                    ) : (
                      <>
                        <h2 className="text-lg font-semibold text-stone-900 mb-2">
                          No analysis yet
                        </h2>
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
                  <div className="space-y-6">
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
                        <div className="flex items-center justify-between mb-3">
                          <h2 className="text-sm font-semibold text-stone-900 opacity-80">
                            {group.label}
                          </h2>
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
            )}

            {activeTab === 'subconscious' && (
              <div className="space-y-6 animate-fade-up">
                {/* Status bar */}
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2 text-xs text-stone-400">
                    {subconsciousEngineStatus && (
                      <>
                        <span>{subconsciousEngineStatus.task_count} tasks</span>
                        <span className="text-stone-300">|</span>
                        <span>{subconsciousEngineStatus.total_ticks} ticks</span>
                        {subconsciousEngineStatus.last_tick_at && (
                          <>
                            <span className="text-stone-300">|</span>
                            <span>
                              Last:{' '}
                              {new Date(
                                subconsciousEngineStatus.last_tick_at * 1000
                              ).toLocaleTimeString()}
                            </span>
                          </>
                        )}
                        {subconsciousEngineStatus.consecutive_failures > 0 && (
                          <>
                            <span className="text-stone-300">|</span>
                            <span className="text-coral-500">
                              {subconsciousEngineStatus.consecutive_failures} failed
                            </span>
                          </>
                        )}
                      </>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    {/* Interval selector */}
                    <div className="flex items-center gap-1.5">
                      <svg
                        className="w-3 h-3 text-stone-400"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                        />
                      </svg>
                      <select
                        value={subconsciousEngineStatus?.interval_minutes ?? 5}
                        onChange={() => {
                          // Config update would require restart — show as read-only for now
                        }}
                        disabled
                        title="Tick interval (change in Settings > Advanced)"
                        className="text-xs bg-stone-50 border border-stone-200 rounded px-1.5 py-0.5 text-stone-500 cursor-not-allowed">
                        <option value={5}>5 min</option>
                        <option value={10}>10 min</option>
                        <option value={15}>15 min</option>
                        <option value={30}>30 min</option>
                        <option value={60}>1 hour</option>
                        <option value={360}>6 hours</option>
                        <option value={720}>12 hours</option>
                        <option value={1440}>1 day</option>
                      </select>
                    </div>
                    <button
                      onClick={triggerTick}
                      disabled={subconsciousTriggering}
                      className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-stone-50 hover:bg-stone-100 disabled:opacity-40 border border-stone-200 rounded-lg text-stone-600 transition-colors">
                      {subconsciousTriggering ? (
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
                            d="M13 10V3L4 14h7v7l9-11h-7z"
                          />
                        </svg>
                      )}
                      Run Now
                    </button>
                  </div>
                </div>

                {/* Escalations — needs user input */}
                {escalations.length > 0 && (
                  <div>
                    <h3 className="text-sm font-semibold text-stone-900 mb-3 flex items-center gap-2">
                      <span className="w-2 h-2 rounded-full bg-amber-400 animate-pulse" />
                      Approval Needed
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-amber-100 text-amber-700">
                        {escalations.length}
                      </span>
                    </h3>
                    <div className="space-y-2">
                      {escalations.map(esc => (
                        <div
                          key={esc.id}
                          className="bg-amber-50 border border-amber-200 rounded-xl p-4">
                          <div className="flex items-start justify-between">
                            <div className="flex-1">
                              <p className="text-sm font-medium text-stone-900">{esc.title}</p>
                              <p className="text-xs text-stone-500 mt-1">{esc.description}</p>
                              <div className="flex items-center gap-2 mt-2">
                                <span
                                  className={`text-[10px] px-2 py-0.5 rounded-full ${
                                    esc.priority === 'critical'
                                      ? 'bg-coral-100 text-coral-700'
                                      : esc.priority === 'important'
                                        ? 'bg-amber-100 text-amber-700'
                                        : 'bg-stone-100 text-stone-600'
                                  }`}>
                                  {esc.priority}
                                </span>
                                <span className="text-[10px] text-stone-400">
                                  Requires your approval to proceed
                                </span>
                              </div>
                            </div>
                            <div className="flex gap-2 ml-3 flex-shrink-0">
                              {isSkillRelated(esc.title, esc.description) ? (
                                <button
                                  onClick={() => {
                                    dismissEscalation(esc.id);
                                    navigate('/skills');
                                  }}
                                  className="px-3 py-1.5 text-xs bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors">
                                  Fix in Skills
                                </button>
                              ) : (
                                <button
                                  onClick={() => approveEscalation(esc.id)}
                                  className="px-3 py-1.5 text-xs bg-sage-500 hover:bg-sage-600 text-white rounded-lg transition-colors">
                                  Go ahead
                                </button>
                              )}
                              <button
                                onClick={() => dismissEscalation(esc.id)}
                                className="px-3 py-1.5 text-xs bg-stone-100 hover:bg-stone-200 text-stone-600 rounded-lg transition-colors">
                                Skip
                              </button>
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* Active tasks */}
                <div>
                  <h3 className="text-sm font-semibold text-stone-900 mb-3">Active Tasks</h3>
                  {subconsciousLoading && subconsciousTasks.length === 0 ? (
                    <div className="text-center py-4">
                      <div className="w-6 h-6 mx-auto border-2 border-stone-300 border-t-transparent rounded-full animate-spin" />
                    </div>
                  ) : subconsciousTasks.filter(t => !t.completed).length === 0 ? (
                    <p className="text-xs text-stone-400 py-3">No active tasks. Add one below.</p>
                  ) : (
                    <div className="space-y-1.5">
                      {/* System tasks — always-on, no controls */}
                      {subconsciousTasks
                        .filter(t => !t.completed && t.source === 'system')
                        .map(task => (
                          <div
                            key={task.id}
                            className="flex items-center py-2 px-3 bg-stone-50 rounded-lg">
                            <div className="w-1.5 h-1.5 rounded-full bg-sage-400 flex-shrink-0 mr-2.5" />
                            <span className="text-sm text-stone-900 truncate flex-1">
                              {task.title}
                            </span>
                            <span className="text-[10px] text-stone-400 flex-shrink-0 px-1.5 py-0.5 rounded bg-stone-100">
                              default
                            </span>
                          </div>
                        ))}
                      {/* User tasks — toggle switch + delete */}
                      {subconsciousTasks
                        .filter(t => !t.completed && t.source !== 'system')
                        .map(task => (
                          <div
                            key={task.id}
                            className="flex items-center justify-between py-2 px-3 bg-stone-50 rounded-lg group">
                            <div className="flex items-center gap-2.5 flex-1 min-w-0">
                              <button
                                onClick={() => toggleSubconsciousTask(task.id, !task.enabled)}
                                className={`relative w-7 h-4 rounded-full flex-shrink-0 transition-colors ${
                                  task.enabled ? 'bg-sage-500' : 'bg-stone-300'
                                }`}>
                                <span
                                  className={`absolute top-0.5 left-0.5 w-3 h-3 rounded-full bg-white shadow transition-transform ${
                                    task.enabled ? 'translate-x-3' : 'translate-x-0'
                                  }`}
                                />
                              </button>
                              <span
                                className={`text-sm truncate ${task.enabled ? 'text-stone-900' : 'text-stone-400'}`}>
                                {task.title}
                              </span>
                            </div>
                            <button
                              onClick={() => removeSubconsciousTask(task.id)}
                              className="opacity-0 group-hover:opacity-100 p-1 text-stone-400 hover:text-coral-500 transition-all">
                              <svg
                                className="w-3.5 h-3.5"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24">
                                <path
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                  strokeWidth={2}
                                  d="M6 18L18 6M6 6l12 12"
                                />
                              </svg>
                            </button>
                          </div>
                        ))}
                    </div>
                  )}

                  {/* Add task */}
                  <form
                    onSubmit={async e => {
                      e.preventDefault();
                      const title = newTaskTitle.trim();
                      if (!title) return;
                      try {
                        await addSubconsciousTask(title);
                        setNewTaskTitle('');
                      } catch {
                        // handled by hook
                      }
                    }}
                    className="flex gap-2 mt-3">
                    <input
                      type="text"
                      placeholder="Add a task... (e.g. 'Check urgent emails')"
                      value={newTaskTitle}
                      onChange={e => setNewTaskTitle(e.target.value)}
                      className="flex-1 px-3 py-2 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50 transition-colors"
                    />
                    <button
                      type="submit"
                      disabled={!newTaskTitle.trim()}
                      className="px-3 py-2 text-sm bg-primary-500 hover:bg-primary-600 disabled:opacity-40 text-white rounded-lg transition-colors">
                      Add
                    </button>
                  </form>
                </div>

                {/* Execution log */}
                <div>
                  <h3 className="text-sm font-semibold text-stone-900 mb-3">Activity Log</h3>
                  {logEntries.length === 0 ? (
                    <p className="text-xs text-stone-400 py-3">
                      No activity yet. Run a tick to see results.
                    </p>
                  ) : (
                    <div className="space-y-1 max-h-64 overflow-y-auto">
                      {logEntries.map(entry => (
                        <div key={entry.id} className="flex items-start gap-2 py-1.5 px-2 text-xs">
                          <span className="text-stone-400 flex-shrink-0 w-14">
                            {new Date(entry.tick_at * 1000).toLocaleTimeString([], {
                              hour: '2-digit',
                              minute: '2-digit',
                            })}
                          </span>
                          <span
                            className={`flex-shrink-0 w-1.5 h-1.5 rounded-full mt-1.5 ${
                              entry.decision === 'act'
                                ? 'bg-sage-400'
                                : entry.decision === 'in_progress'
                                  ? 'bg-primary-400 animate-pulse'
                                  : entry.decision === 'escalate'
                                    ? 'bg-amber-400'
                                    : entry.decision === 'failed'
                                      ? 'bg-coral-400'
                                      : entry.decision === 'cancelled'
                                        ? 'bg-stone-300'
                                        : entry.decision === 'dismissed'
                                          ? 'bg-stone-300'
                                          : 'bg-stone-200'
                            }`}
                          />
                          <span
                            className={`break-words min-w-0 ${
                              entry.decision === 'in_progress'
                                ? 'text-stone-400'
                                : entry.decision === 'failed'
                                  ? 'text-coral-500'
                                  : 'text-stone-600'
                            } ${entry.result && entry.result.length > 120 ? 'cursor-pointer hover:text-stone-900' : ''}`}
                            onClick={() => {
                              if (entry.result && entry.result.length > 120) {
                                setExpandedLogIds(prev => {
                                  const next = new Set(prev);
                                  if (next.has(entry.id)) next.delete(entry.id);
                                  else next.add(entry.id);
                                  return next;
                                });
                              }
                            }}>
                            {entry.result
                              ? expandedLogIds.has(entry.id)
                                ? entry.result
                                : entry.result.length > 120
                                  ? `${entry.result.substring(0, 120)}...`
                                  : entry.result
                              : entry.decision === 'noop'
                                ? 'Nothing new'
                                : entry.decision === 'act'
                                  ? 'Completed'
                                  : entry.decision === 'in_progress'
                                    ? 'Evaluating...'
                                    : entry.decision === 'escalate'
                                      ? 'Waiting for approval'
                                      : entry.decision === 'failed'
                                        ? 'Failed'
                                        : entry.decision === 'cancelled'
                                          ? 'Cancelled'
                                          : entry.decision === 'dismissed'
                                            ? 'Skipped'
                                            : entry.decision}
                          </span>
                          {entry.duration_ms != null && (
                            <span className="text-stone-300 flex-shrink-0 ml-auto">
                              {entry.duration_ms > 1000
                                ? `${(entry.duration_ms / 1000).toFixed(1)}s`
                                : `${entry.duration_ms}ms`}
                            </span>
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            )}

            {activeTab === 'dreams' && (
              <div className="glass rounded-2xl p-8 text-center animate-fade-up">
                <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-sky-500/10">
                  <svg
                    className="w-8 h-8 text-sky-400"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={1.5}
                      d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178z"
                    />
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={1.5}
                      d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                    />
                  </svg>
                </div>
                <h2 className="text-lg font-semibold text-stone-900 mb-2">Dreams</h2>
                <p className="text-stone-400 text-sm mb-1">
                  Twice everyday, OpenHuman will generate a dream (or a summary) based on everything
                  that has happened in your life today. These dreams re then indexed and can be used
                  to influence OpenHuman's behavior.
                </p>
                <p className="text-xs text-stone-500">Coming soon</p>
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
