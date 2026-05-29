import { useCallback, useEffect, useState } from 'react';

import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import DiagramViewerTab from '../components/intelligence/DiagramViewerTab';
import GraphCentralityTab from '../components/intelligence/GraphCentralityTab';
import IntelligenceCallsTab from '../components/intelligence/IntelligenceCallsTab';
import IntelligenceDreamsTab from '../components/intelligence/IntelligenceDreamsTab';
import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import IntelligenceTasksTab from '../components/intelligence/IntelligenceTasksTab';
import { MemoryWorkspace } from '../components/intelligence/MemoryWorkspace';
import { ToastContainer } from '../components/intelligence/Toast';
import PillTabBar from '../components/PillTabBar';
import {
  useIntelligenceSocket,
  useIntelligenceSocketManager,
} from '../hooks/useIntelligenceSocket';
import { useSubconscious } from '../hooks/useSubconscious';
import { useT } from '../lib/i18n/I18nContext';
import type {
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../types/intelligence';

type IntelligenceTab =
  | 'memory'
  | 'subconscious'
  | 'calls'
  | 'dreams'
  | 'tasks'
  | 'diagram'
  | 'centrality';

export default function Intelligence() {
  const { t } = useT();

  const [activeTab, setActiveTab] = useState<IntelligenceTab>('memory');

  // The legacy header pills (system-status + Ingesting/Queued chips) were
  // sourced from `useConsciousItems` + `useMemoryIngestionStatus`. They are
  // replaced by the Memory Tree status panel (#1856 Part 1), rendered inside
  // `MemoryWorkspace`, which polls `memory_tree_pipeline_status` for a much
  // richer dashboard. The hooks themselves still exist for any future
  // consumers / tests; we just no longer feed them into a half-baked pill
  // up here.

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

  // Initialize socket connection
  useEffect(() => {
    if (!socketConnected) {
      socketManager.connect();
    }
  }, [socketConnected, socketManager]);

  const tabs: { id: IntelligenceTab; label: string; comingSoon?: boolean }[] = [
    { id: 'memory', label: t('memory.tab.memory') },
    { id: 'subconscious', label: t('memory.tab.subconscious') },
    { id: 'tasks', label: 'Tasks' },
    { id: 'diagram', label: t('memory.tab.diagram') },
    { id: 'calls', label: t('memory.tab.calls') },
    { id: 'dreams', label: t('memory.tab.dreams') },
    { id: 'centrality', label: t('memory.tab.centrality') },
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
                        : 'border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 text-stone-500 dark:text-neutral-400'
                    }`}>
                    {t('misc.beta')}
                  </span>
                )}
              </span>
            );
          }}
        />

        <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6">
          <div>
            {/* Header */}
            <div className="flex items-center justify-between mb-6">
              <div className="flex items-center gap-3">
                <h1
                  className="text-xl font-bold text-stone-900 dark:text-neutral-100"
                  data-walkthrough="intelligence-header">
                  {t('memory.title')}
                </h1>
                {/* Header count badge was sourced from `stats.total` which
                    in turn came from the legacy actionable-items pipeline
                    (`filterItems(items, ...)`). The Memory tab now mounts
                    `MemoryWorkspace`, which renders chunks from
                    `memory_tree` and has nothing to do with that pipeline,
                    so the badge would have shown a count that no longer
                    matches anything visible. Hidden until a memory_tree
                    -native count signal is exposed. */}
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

            {activeTab === 'tasks' && <IntelligenceTasksTab />}

            {activeTab === 'diagram' && <DiagramViewerTab />}

            {activeTab === 'calls' && <IntelligenceCallsTab onToast={addToast} />}

            {activeTab === 'dreams' && <IntelligenceDreamsTab />}

            {activeTab === 'centrality' && <GraphCentralityTab />}
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
