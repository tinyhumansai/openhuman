/**
 * WorkflowsTab
 * ------------
 *
 * The Intelligence page's "Workflows" tab — the single home for installed
 * workflows (the unified primitive: a goal + the procedure to reach it,
 * authored as SKILL.md bundles and served by the `workflows_*` JSON-RPC via
 * `workflowsApi`).
 *
 * Owns the full workflow surface that used to live on the Connections page:
 *   - lists discovered workflows as cards,
 *   - opens a detail drawer (with a "Run workflow" CTA → /skills/run),
 *   - create / install-from-URL / uninstall flows.
 *
 * Workflows are intentionally NOT shown on Connections anymore — Connections
 * is for integrations (Composio / channels / MCP); workflows are an
 * intelligence concern.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { workflowsApi, type WorkflowSummary } from '../../services/api/workflowsApi';
import type { ToastNotification } from '../../types/intelligence';
import SettingsPanel from '../settings/layout/SettingsPanel';
import CreateSkillModal from '../skills/CreateSkillModal';
import UnifiedSkillCard from '../skills/SkillCard';
import { BUILT_IN_SKILL_ICONS } from '../skills/skillIcons';
import UninstallSkillConfirmDialog from '../skills/UninstallSkillConfirmDialog';
import Button from '../ui/Button';
import { ToastContainer } from './Toast';

const log = debug('intelligence:workflows');

interface WorkflowsTabProps {
  /**
   * When true, render with the standard Settings chrome ({@link SettingsPanel}):
   * a "Workflows" title, the subtitle as the description, and "New workflow" in
   * the header action slot. The page tab hosts (Activity, Intelligence) leave
   * this off and keep the inline subtitle + button row above the list.
   */
  asSettingsPanel?: boolean;
}

export default function WorkflowsTab({ asSettingsPanel = false }: WorkflowsTabProps = {}) {
  const { t } = useT();
  const navigate = useNavigate();
  const [workflows, setWorkflows] = useState<WorkflowSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [uninstallCandidate, setUninstallCandidate] = useState<WorkflowSummary | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);

  const refresh = useCallback(async (): Promise<WorkflowSummary[]> => {
    try {
      const list = await workflowsApi.listWorkflows();
      log('listWorkflows ok count=%d', list.length);
      setLoadError(null);
      setWorkflows(list);
      return list;
    } catch (err) {
      // Don't let a backend failure masquerade as "no workflows installed" —
      // surface it as an error/retry state instead of the empty state.
      const message = err instanceof Error ? err.message : String(err);
      log('listWorkflows error %s', message);
      setLoadError(message);
      return [];
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const list = await refresh();
      if (cancelled) return;
      void list;
    })();
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  const isEmpty = workflows.length === 0 && !loading && !loadError;

  const newWorkflowButton = (
    <Button
      variant="primary"
      size="sm"
      data-testid="workflows-create-btn"
      onClick={() => setCreateModalOpen(true)}>
      {t('workflows.createNew')}
    </Button>
  );

  const body = (
    <>
      {/* Load error — shown instead of the empty state when listWorkflows fails,
          so an outage doesn't read as "you have no workflows". */}
      {loadError && !loading ? (
        <div
          data-testid="workflows-load-error"
          className="rounded-2xl border border-coral-200 dark:border-coral-800 bg-coral-50 dark:bg-coral-950/40 p-4 text-center shadow-soft animate-fade-up">
          <h2 className="text-sm font-semibold text-coral-800 dark:text-coral-200">
            {t('common.error')}
          </h2>
          <p className="mt-1 break-words font-mono text-[11px] text-coral-700/90 dark:text-coral-300/90">
            {loadError}
          </p>
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            className="mt-3"
            onClick={() => {
              setLoading(true);
              void refresh();
            }}>
            {t('common.retry')}
          </Button>
        </div>
      ) : null}

      {/* Loading skeleton */}
      {loading && workflows.length === 0 ? (
        <div className="space-y-2 animate-pulse" data-testid="workflows-loading">
          {[1, 2, 3].map(i => (
            <div key={i} className="h-20 rounded-2xl bg-surface-subtle" />
          ))}
        </div>
      ) : null}

      {/* Empty state */}
      {isEmpty ? (
        <div className="rounded-2xl border border-line bg-surface p-10 text-center shadow-soft animate-fade-up">
          <h2 className="text-sm font-semibold text-content">{t('workflows.empty.title')}</h2>
          <p className="mt-1 text-xs text-content-muted">{t('workflows.empty.body')}</p>
          <Button
            variant="primary"
            size="md"
            className="mt-4"
            onClick={() => setCreateModalOpen(true)}>
            {t('workflows.createNew')}
          </Button>
        </div>
      ) : null}

      {/* Workflow list */}
      {workflows.length > 0 ? (
        <div
          className="rounded-2xl border border-line bg-surface p-3 shadow-soft animate-fade-up"
          data-testid="workflows-list">
          <div className="space-y-2">
            {workflows.map(wf => {
              const scopeLabel = wf.legacy
                ? t('scope.legacy')
                : wf.scope === 'user'
                  ? t('scope.user')
                  : wf.scope === 'project'
                    ? t('scope.project')
                    : t('scope.legacy');
              const scopeColor = wf.legacy
                ? 'text-content-secondary'
                : wf.scope === 'user'
                  ? 'text-sage-600'
                  : wf.scope === 'project'
                    ? 'text-amber-600'
                    : 'text-content-secondary';
              const canUninstall = wf.scope === 'user' && !wf.legacy;
              return (
                <UnifiedSkillCard
                  key={wf.id}
                  icon={BUILT_IN_SKILL_ICONS.screenIntelligence}
                  title={wf.name}
                  description={wf.description}
                  statusLabel={scopeLabel}
                  statusColor={scopeColor}
                  ctaLabel={t('common.seeAll')}
                  testId={`workflow-card-${wf.id}`}
                  ctaTestId={`workflow-open-${wf.id}`}
                  onCtaClick={() => {
                    log('open runner workflowId=%s', wf.id);
                    navigate(`/workflows/run?workflow=${encodeURIComponent(wf.id)}&lock=1`);
                  }}
                  secondaryActions={
                    canUninstall
                      ? [
                          {
                            label: t('workflows.delete'),
                            testId: `workflow-uninstall-${wf.id}`,
                            icon: (
                              <svg
                                className="h-3.5 w-3.5"
                                fill="none"
                                stroke="currentColor"
                                strokeWidth="2"
                                viewBox="0 0 24 24">
                                <path
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                  d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14z"
                                />
                              </svg>
                            ),
                            onClick: () => setUninstallCandidate(wf),
                          },
                        ]
                      : undefined
                  }
                />
              );
            })}
          </div>
        </div>
      ) : null}

      {/* Create modal (New workflow). Editing a workflow now happens on the
          runner page it opens into. */}
      {createModalOpen && (
        <CreateSkillModal
          onClose={() => setCreateModalOpen(false)}
          onCreated={wf => {
            log('created workflowId=%s', wf.id);
            setCreateModalOpen(false);
            setWorkflows(prev => (prev.some(s => s.id === wf.id) ? prev : [...prev, wf]));
            void refresh();
            // Land the user on the new workflow's runner page.
            navigate(`/workflows/run?workflow=${encodeURIComponent(wf.id)}&lock=1`);
          }}
        />
      )}

      {/* Uninstall confirmation */}
      {uninstallCandidate && (
        <UninstallSkillConfirmDialog
          skill={uninstallCandidate}
          onClose={() => setUninstallCandidate(null)}
          onUninstalled={result => {
            log('uninstalled name=%s', result.name);
            // Reconcile by the workflow's id/slug — the uninstall flow is keyed
            // by `skill.id`, which can diverge from the display `name`. Keying
            // the filter off `result.name` would leave the card rendered when
            // they differ (and never clear if the refetch fails).
            const removedId = uninstallCandidate.id;
            addToast({
              type: 'success',
              title: t('workflows.delete'),
              message: `"${result.name}" ${t('common.success')}`,
            });
            setWorkflows(prev => prev.filter(s => s.id !== removedId));
            void refresh();
          }}
        />
      )}

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </>
  );

  // Settings route: full panel chrome (title from the route registry, subtitle
  // as description, "New workflow" in the header action slot).
  if (asSettingsPanel) {
    return (
      <SettingsPanel description={t('workflows.subtitle')} action={newWorkflowButton}>
        {body}
      </SettingsPanel>
    );
  }

  // Page tab hosts (Activity / Intelligence): inline subtitle + button row above
  // the list, since those pages own the surrounding chrome.
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <p className="min-w-0 text-xs text-content-muted">{t('workflows.subtitle')}</p>
        <div className="flex flex-shrink-0 items-center gap-2">{newWorkflowButton}</div>
      </div>
      {body}
    </div>
  );
}
