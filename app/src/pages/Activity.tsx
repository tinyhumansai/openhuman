import { useCallback, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import { ToastContainer } from '../components/intelligence/Toast';
import WorkflowsTab from '../components/intelligence/WorkflowsTab';
import ChipTabs from '../components/layout/ChipTabs';
import PageSectionHeader from '../components/layout/PageSectionHeader';
import {
  useIntelligenceSocket,
  useIntelligenceSocketManager,
} from '../hooks/useIntelligenceSocket';
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
type ActivityTab = 'automations' | 'alerts';

const ACTIVITY_TABS: ActivityTab[] = ['automations', 'alerts'];

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
    { id: 'alerts', label: t('activity.tabs.alerts') },
  ];
  const activeTabDef = tabs.find(tab => tab.id === activeTab);

  return (
    // `p-4 pt-6` is the accepted top-level page inset (Notifications and Invites
    // use the same), and `max-w-3xl` is the contentWidth `lg` step — the column
    // used to be a hand-picked `max-w-4xl`, which is on no scale.
    <div className="min-h-full space-y-4 p-4 pt-6">
      <ChipTabs<ActivityTab>
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
        // The page owns its own gutter, so the row drops ChipTabs' default
        // padding and keeps only the canonical chip gap — `gap-1.5`, not the
        // `gap-2 pb-1` this hand-picked before.
        className="mx-auto flex max-w-3xl flex-wrap gap-1.5"
      />

      {/* Alerts hands the surface to the Notifications page, which brings its
          own header card and per-section widths — so it renders outside this
          page's content column and without the tab header above it. */}
      {activeTab === 'alerts' ? (
        <Notifications />
      ) : (
        // No card frame here: every routed page already sits inside
        // `ContentSurface`'s opaque sheet, so wrapping the whole body in
        // `rounded-2xl border bg-surface shadow-soft` was a card on a card.
        // `PageSectionHeader` is the canonical header for a page view that is
        // not a PanelPage — the same shape Notifications uses one level down.
        <div className="mx-auto max-w-3xl space-y-4">
          {/* Header — reflects the active tab so the panel title matches
              what's shown below it, rather than a static "Activity". */}
          <PageSectionHeader
            title={
              <span data-walkthrough="intelligence-header">
                {activeTabDef?.label ?? t('nav.activity')}
              </span>
            }
            description={activeTabDef?.description}
          />

          {/* Tab content */}
          {activeTab === 'automations' && <WorkflowsTab />}
        </div>
      )}

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
