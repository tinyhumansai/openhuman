import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import IntelligenceAgentsTab from '../components/intelligence/IntelligenceAgentsTab';
import IntelligenceAgentWorkTab from '../components/intelligence/IntelligenceAgentWorkTab';
import IntelligenceOrchestrationTab from '../components/intelligence/IntelligenceOrchestrationTab';
import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import IntelligenceTasksTab from '../components/intelligence/IntelligenceTasksTab';
import IntelligenceTeamsTab from '../components/intelligence/IntelligenceTeamsTab';
import IntelligenceWorktreesTab from '../components/intelligence/IntelligenceWorktreesTab';
import MemorySection from '../components/intelligence/MemorySection';
import ModelCouncilTab from '../components/intelligence/ModelCouncilTab';
import { ToastContainer } from '../components/intelligence/Toast';
import WorkflowsTab from '../components/intelligence/WorkflowsTab';
import ChipTabs from '../components/layout/ChipTabs';
import SettingsHeader from '../components/settings/components/SettingsHeader';
import { useSettingsNavigation } from '../components/settings/hooks/useSettingsNavigation';
import { useDeveloperMode } from '../hooks/useDeveloperMode';
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

const log = debug('settings:intelligence');

type IntelligenceTab =
  | 'memory'
  | 'subconscious'
  | 'tasks'
  | 'agent-work'
  | 'worktrees'
  | 'teams'
  | 'agents'
  | 'workflows'
  | 'orchestration'
  | 'council';

const INTELLIGENCE_TABS: IntelligenceTab[] = [
  'memory',
  'subconscious',
  'tasks',
  'agent-work',
  'worktrees',
  'teams',
  'agents',
  'workflows',
  'orchestration',
  'council',
];

// Tabs gated to dev builds or runtime developer mode (mirrors the `devOnly`
// flags on `allTabs` below). A `?tab=` deep link must be validated against the
// *visible* set, not the raw enum, so a user cannot force-open a hidden tab.
const DEV_ONLY_TABS: IntelligenceTab[] = ['council'];

const makeIsVisibleTab =
  (developerModeEnabled: boolean) =>
  (tab: string | null | undefined): tab is IntelligenceTab =>
    (INTELLIGENCE_TABS as string[]).includes(tab ?? '') &&
    (developerModeEnabled || !(DEV_ONLY_TABS as string[]).includes(tab ?? ''));

interface IntelligenceProps {
  /**
   * Query-param key backing the active tab. Defaults to `tab` for the standalone
   * route. When embedded inside another `?tab=`-driven page (e.g. Brain at
   * `/brain?tab=intelligence`), pass a distinct key so the child's internal tab
   * switches don't clobber the host's `tab` param and unmount this panel.
   */
  tabParamKey?: string;
}

export default function Intelligence({ tabParamKey = 'tab' }: IntelligenceProps = {}) {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const developerMode = useDeveloperMode();
  const isVisibleTab = makeIsVisibleTab(developerMode);

  // [settings] Intelligence is rendered exclusively at /settings/intelligence.
  // Always apply the settings shell (SettingsHeader + breadcrumbs) — no need
  // for an `embedded` prop since this page has no standalone usage.
  log('rendering with settings shell');

  // Tab is URL-backed (`?<tabParamKey>=…`) so navigating away — e.g. to
  // Settings → Task Sources from the Agent Tasks tab — and coming back via
  // browser-back restores the same tab instead of resetting to Memory.
  // `replace` so switching tabs doesn't stack history entries.
  const [searchParams, setSearchParams] = useSearchParams();
  const tabParam = searchParams.get(tabParamKey);
  const activeTab: IntelligenceTab = isVisibleTab(tabParam) ? tabParam : 'tasks';
  const setActiveTab = useCallback(
    (tab: IntelligenceTab) => {
      setSearchParams(
        prev => {
          prev.set(tabParamKey, tab);
          return prev;
        },
        { replace: true }
      );
    },
    [setSearchParams, tabParamKey]
  );

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
    status: subconsciousEngineStatus,
    mode: subconsciousMode,
    intervalMinutes: subconsciousInterval,
    triggering: subconsciousTriggering,
    settingMode: subconsciousSettingMode,
    triggerTick,
    setMode: setSubconsciousMode,
    setIntervalMinutes: setSubconsciousInterval,
  } = useSubconscious();

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

  const allTabs: {
    id: IntelligenceTab;
    label: string;
    description?: string;
    comingSoon?: boolean;
    devOnly?: boolean;
  }[] = [
    { id: 'tasks', label: t('memory.tab.tasks'), description: t('memory.tab.tasksDescription') },
    {
      id: 'agent-work',
      label: t('memory.tab.agentWork'),
      description: t('memory.tab.agentWorkDescription'),
    },
    {
      id: 'worktrees',
      label: t('memory.tab.worktrees'),
      description: t('memory.tab.worktreesDescription'),
    },
    { id: 'teams', label: t('memory.tab.teams'), description: t('memory.tab.teamsDescription') },
    { id: 'memory', label: t('memory.tab.memory') },
    { id: 'subconscious', label: t('memory.tab.subconscious') },
    {
      id: 'workflows',
      label: t('memory.tab.workflows'),
      description: t('memory.tab.workflowsDescription'),
    },
    {
      id: 'orchestration',
      label: t('memory.tab.orchestration'),
      description: t('memory.tab.orchestrationDescription'),
    },
    { id: 'council', label: t('memory.tab.council'), devOnly: true },
    { id: 'agents', label: t('memory.tab.agents'), description: t('memory.tab.agentsDescription') },
  ];
  const tabs = allTabs.filter(tab => !tab.devOnly || developerMode);
  const activeTabDef = tabs.find(tab => tab.id === activeTab);

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.developerMenu.intelligence.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <ChipTabs<IntelligenceTab>
          items={tabs.map(tab => ({
            id: tab.id,
            label: (
              <span className="inline-flex items-center gap-1.5">
                <span>{tab.label}</span>
                {tab.comingSoon && (
                  <span className="rounded-full border border-line bg-surface-muted px-1.5 py-0.5 text-[10px] text-content-muted">
                    {t('misc.beta')}
                  </span>
                )}
              </span>
            ),
          }))}
          value={activeTab}
          onChange={setActiveTab}
          className="flex flex-wrap gap-2 pb-1"
        />

        <div className="bg-surface rounded-2xl shadow-soft border border-line p-6">
          <div>
            {/* Sub-heading — reflects the active tab (e.g. "Agent Tasks") so
                the panel body title matches what's shown below it, rather than
                a static page title. The top-level title is now in SettingsHeader. */}
            <div className="flex items-center justify-between mb-6">
              <div className="min-w-0">
                <h2
                  className="text-xl font-bold text-content"
                  data-walkthrough="intelligence-header">
                  {activeTabDef?.label ?? t('memory.title')}
                </h2>
                {activeTabDef?.description && (
                  <p className="mt-1 text-sm text-content-muted">{activeTabDef.description}</p>
                )}
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
            {activeTab === 'memory' && <MemorySection onToast={addToast} />}

            {activeTab === 'subconscious' && (
              <IntelligenceSubconsciousTab
                status={subconsciousEngineStatus}
                mode={subconsciousMode}
                intervalMinutes={subconsciousInterval}
                triggerTick={triggerTick}
                triggering={subconsciousTriggering}
                settingMode={subconsciousSettingMode}
                setMode={setSubconsciousMode}
                setIntervalMinutes={setSubconsciousInterval}
              />
            )}

            {activeTab === 'tasks' && <IntelligenceTasksTab />}

            {activeTab === 'agent-work' && <IntelligenceAgentWorkTab />}

            {activeTab === 'worktrees' && <IntelligenceWorktreesTab />}

            {activeTab === 'teams' && <IntelligenceTeamsTab />}

            {activeTab === 'agents' && <IntelligenceAgentsTab />}

            {activeTab === 'workflows' && <WorkflowsTab />}

            {activeTab === 'orchestration' && <IntelligenceOrchestrationTab />}

            {activeTab === 'council' && <ModelCouncilTab />}
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
