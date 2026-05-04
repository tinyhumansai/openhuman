import { useCallback, useEffect, useMemo, useState } from 'react';

import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import IntelligenceDreamsTab from '../components/intelligence/IntelligenceDreamsTab';
import IntelligenceSettingsTab from '../components/intelligence/IntelligenceSettingsTab';
import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import { MemoryWorkspace } from '../components/intelligence/MemoryWorkspace';
import { ToastContainer } from '../components/intelligence/Toast';
import { filterItems, getItemStats } from '../components/intelligence/utils';
import PillTabBar from '../components/PillTabBar';
import { useConsciousItems } from '../hooks/useConsciousItems';
import {
  useIntelligenceSocket,
  useIntelligenceSocketManager,
} from '../hooks/useIntelligenceSocket';
import { useIntelligenceStats } from '../hooks/useIntelligenceStats';
import { useMemoryIngestionStatus } from '../hooks/useMemoryIngestionStatus';
import { useScreenIntelligenceItems } from '../hooks/useScreenIntelligenceItems';
import { useSubconscious } from '../hooks/useSubconscious';
import type {
  ActionableItem,
  ActionableItemSource,
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../types/intelligence';

type IntelligenceTab = 'memory' | 'subconscious' | 'dreams' | 'settings';

export default function Intelligence() {
  const { aiStatus } = useIntelligenceStats();
  const { status: ingestionStatus } = useMemoryIngestionStatus();

  const [activeTab, setActiveTab] = useState<IntelligenceTab>('memory');
  // The actionable-card filters (source / priority / search) are inert
  // now that IntelligenceMemoryTab is gone — the new MemoryWorkspace owns
  // its own search + lens state. Kept as constants so the legacy
  // filterItems pipeline that feeds the header count badge still has the
  // shape it expects, without exposing setters that nothing calls.
  const sourceFilter: ActionableItemSource | 'all' = 'all';
  const priorityFilter: 'critical' | 'important' | 'normal' | 'all' = 'all';
  const searchFilter = '';

  // Conscious memory items (real data from the background analysis loop)
  const {
    items: consciousItems,
    loading: consciousLoading,
    isRunning,
  } = useConsciousItems();

  // useUpdateActionableItem / useSnoozeActionableItem hooks were the
  // mutations behind handleComplete / Dismiss / Snooze. Removed along
  // with those handlers since the Memory tab no longer renders the
  // actionable-card surface.

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

  const stats = useMemo(() => getItemStats(filteredItems), [filteredItems]);

  // Item-action handlers (handleComplete / Dismiss / Snooze / AnalyzeNow)
  // were wired to the legacy IntelligenceMemoryTab's actionable cards,
  // which the Memory tab no longer renders (it now mounts MemoryWorkspace
  // — a memory_tree-backed browser). The cards' update / snooze flows
  // live in IntelligenceSubconsciousTab + IntelligenceDreamsTab via
  // their own hooks, so Intelligence.tsx no longer needs these closures.
  // Removed to satisfy noUnusedLocals; reintroduce if a future tab
  // surfaces actionable-card editing again.

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
    { id: 'settings', label: 'Settings' },
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
                {activeTab === 'memory' &&
                  (ingestionStatus.running || ingestionStatus.queueDepth > 0) && (
                    <div
                      className="flex items-center gap-1.5 px-2 py-0.5 rounded-full border border-amber-200 bg-amber-50 text-amber-700"
                      title={
                        ingestionStatus.running
                          ? ingestionStatus.currentTitle
                            ? `Ingesting: ${ingestionStatus.currentTitle}`
                            : 'Memory ingestion running'
                          : 'Memory ingestion queued'
                      }>
                      <div className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse" />
                      <span className="text-[11px] font-medium">
                        {ingestionStatus.running ? 'Ingesting' : 'Queued'}
                        {ingestionStatus.queueDepth > 0 && ` · ${ingestionStatus.queueDepth}`}
                      </span>
                    </div>
                  )}
                {/* Analyze Now / Refresh button removed — the new
                    MemoryWorkspace fetches via memory_tree RPCs that
                    don't need a manual trigger. The actionable-cards
                    flow (handleAnalyzeNow) is no longer reachable from
                    the Memory tab; left in scope only for the legacy
                    subconscious/dreams tabs that still use it. */}
              </div>
            </div>

            {/* Tab content */}
            {activeTab === 'memory' && <MemoryWorkspace onToast={addToast} />}

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

            {activeTab === 'settings' && <IntelligenceSettingsTab />}
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
