import { useCallback, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import { ToastContainer } from '../components/intelligence/Toast';
import WorkflowsTab from '../components/intelligence/WorkflowsTab';
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
import Notifications from './Notifications';

// Visible tab IDs for the Activity surface.
// memory, agents, council and tasks have moved to Settings → Developer & Diagnostics
// (routes: /settings/intelligence, /settings/agents, /settings/tasks).
// Back-compat: ?tab=memory / ?tab=agents / ?tab=council / ?tab=tasks are unknown
// to the visible set and therefore fall back to 'automations' (see isVisibleTab).
type ActivityTab = 'automations' | 'backgroundActivity' | 'alerts';

const ACTIVITY_TABS: ActivityTab[] = ['automations', 'backgroundActivity', 'alerts'];

/**
 * Returns a type-guard predicate for the currently visible tabs.
 * Unknown values (including old deep-link tabs like ?tab=memory or ?tab=tasks)
 * fall back to the default tab rather than erroring.
 */
const isVisibleTab = (tab: string | null | undefined): tab is ActivityTab =>
  (ACTIVITY_TABS as string[]).includes(tab ?? '');

export default function Activity() {
  const { t } = useT();

  // Tab is URL-backed (/activity?tab=…) so navigating away and coming back
  // restores the same tab.  `replace` so switching tabs doesn't stack history.
  const [searchParams, setSearchParams] = useSearchParams();
  const tabParam = searchParams.get('tab');
  const activeTab: ActivityTab = isVisibleTab(tabParam) ? tabParam : 'automations';
  const setActiveTab = useCallback(
    (tab: ActivityTab) => {
      setSearchParams(
        prev => {
          prev.set('tab', tab);
          return prev;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  // Subconscious engine data (used by the Background Activity tab).
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

  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);

  // Initialize socket connection
  useEffect(() => {
    if (!socketConnected) {
      socketManager.connect();
    }
  }, [socketConnected, socketManager]);

  const tabs: { id: ActivityTab; label: string; description?: string; comingSoon?: boolean }[] = [
    {
      id: 'automations',
      label: t('activity.tabs.automations'),
      description: t('activity.tabs.automationsDescription'),
    },
    { id: 'backgroundActivity', label: t('activity.tabs.backgroundActivity') },
    { id: 'alerts', label: t('activity.tabs.alerts') },
  ];
  const activeTabDef = tabs.find(tab => tab.id === activeTab);

  return (
    <div className="min-h-full p-4 pt-6">
      <div className="max-w-4xl mx-auto space-y-4">
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

        {/* Alerts tab renders outside the card so Notifications can use its own
            full-width layout with multiple sections. */}
        {activeTab === 'alerts' ? (
          <Notifications />
        ) : (
          <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6">
            <div>
              {/* Header — reflects the active tab so the panel title matches
                  what's shown below it, rather than a static "Activity". */}
              <div className="flex items-center justify-between mb-6">
                <div className="min-w-0">
                  <h1
                    className="text-xl font-bold text-stone-900 dark:text-neutral-100"
                    data-walkthrough="intelligence-header">
                    {activeTabDef?.label ?? t('nav.activity')}
                  </h1>
                  {activeTabDef?.description && (
                    <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">
                      {activeTabDef.description}
                    </p>
                  )}
                </div>
              </div>

              {/* Tab content */}
              {activeTab === 'automations' && <WorkflowsTab />}

              {activeTab === 'backgroundActivity' && (
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
            </div>
          </div>
        )}
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
