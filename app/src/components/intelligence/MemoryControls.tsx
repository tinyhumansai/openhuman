/**
 * MemoryControls — the unified action toolbar for the memory surface.
 *
 * Extracted from `MemoryWorkspace` so the Brain page and Settings → Memory
 * share one elegant, consistent control bar instead of a row of mismatched,
 * differently-coloured buttons.
 *
 * Visual language (deliberate hierarchy, not a rainbow):
 *   - Trees / Contacts — a segmented graph-mode toggle on the left.
 *   - Build summary trees — the single primary (filled) action.
 *   - Refresh · View vault — quiet ghost buttons.
 *   - Reset memory · Reset memory tree — destructive, intentionally muted and
 *     set apart behind a divider; they only reveal their warning tint on hover.
 *
 * The parent owns the graph fetch: every mutation (and Refresh) calls
 * `onRefresh()` so the caller re-pulls the graph on its own cadence.
 */
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import {
  type GraphMode,
  memoryTreeFlushNow,
  memoryTreeResetTree,
  memoryTreeWipeAll,
} from '../../utils/tauriCommands';
import ChipTabs from '../layout/ChipTabs';
import Button from '../ui/Button';
import Separator from '../ui/Separator';
import { ObsidianVaultSection } from './ObsidianVaultSection';

interface MemoryControlsProps {
  mode: GraphMode;
  onModeChange: (next: GraphMode) => void;
  /** Re-pull the graph — parent owns the fetch. Called after every mutation. */
  onRefresh: () => void;
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
  /** Absolute content root (from graph export); enables the View vault button. */
  contentRootAbs?: string | null;
}

export function MemoryControls({
  mode,
  onModeChange,
  onRefresh,
  onToast,
  contentRootAbs,
}: MemoryControlsProps) {
  const { t } = useT();
  const [building, setBuilding] = useState(false);
  const [wiping, setWiping] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const busy = building || wiping || resetting;

  const handleWipe = useCallback(async () => {
    // Two-step confirm so accidental clicks can't nuke a workspace.
    if (!window.confirm(t('workspace.wipeConfirm'))) return;
    setWiping(true);
    try {
      const resp = await memoryTreeWipeAll();
      onToast?.({
        type: 'success',
        title: t('workspace.wipeSuccessTitle'),
        message: t('workspace.wipeSuccessMessage')
          .replace('{rows}', resp.rows_deleted.toLocaleString())
          .replace('{dirs}', String(resp.dirs_removed.length))
          .replace('{cursors}', resp.sync_state_cleared.toLocaleString()),
      });
      // Re-pull immediately so the canvas reflects the wipe.
      onRefresh();
    } catch (err) {
      console.error('[ui-flow][memory-controls] wipe_all failed', err);
      onToast?.({
        type: 'error',
        title: t('workspace.wipeFailedTitle'),
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setWiping(false);
    }
  }, [onToast, onRefresh, t]);

  const handleResetTree = useCallback(async () => {
    if (!window.confirm(t('workspace.resetTreeConfirm'))) return;
    setResetting(true);
    try {
      const resp = await memoryTreeResetTree();
      onToast?.({
        type: 'success',
        title: t('workspace.resetTreeSuccessTitle'),
        message: t('workspace.resetTreeSuccessMessage')
          .replace('{treeRows}', resp.tree_rows_deleted.toLocaleString())
          .replace('{chunks}', resp.chunks_requeued.toLocaleString())
          .replace('{jobs}', resp.jobs_enqueued.toLocaleString()),
      });
      // reset_tree restarts from extract jobs (slower than seal-only) — give the
      // worker a longer head start than build does before re-pulling.
      setTimeout(() => onRefresh(), 8000);
    } catch (err) {
      console.error('[ui-flow][memory-controls] reset_tree failed', err);
      onToast?.({
        type: 'error',
        title: t('workspace.resetTreeFailedTitle'),
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setResetting(false);
    }
  }, [onToast, onRefresh, t]);

  const handleBuildTrees = useCallback(async () => {
    setBuilding(true);
    try {
      const resp = await memoryTreeFlushNow();
      onToast?.({
        type: resp.enqueued ? 'success' : 'info',
        title: resp.enqueued
          ? `Building summary trees · ${resp.stale_buffers} buffer(s)`
          : 'Build already in progress',
        message: resp.enqueued
          ? 'Force-sealing every L0 buffer through the configured AI summariser. The graph will refresh once the worker drains.'
          : 'A flush job for today is already queued — no new work needed.',
      });
      // The seal cascade runs async on the worker pool; 4s covers the typical
      // case without making the UI feel stuck.
      setTimeout(() => onRefresh(), 4000);
    } catch (err) {
      console.error('[ui-flow][memory-controls] flush_now failed', err);
      onToast?.({
        type: 'error',
        title: t('workspace.buildTreesFailedTitle'),
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setBuilding(false);
    }
  }, [onToast, onRefresh, t]);

  const handleRefresh = useCallback(async () => {
    if (refreshing) return;
    console.debug('[ui-flow][memory-controls] manual refresh: entry');
    setRefreshing(true);
    try {
      // `onRefresh` re-pulls the graph; the parent owns the fetch and may
      // resolve synchronously (it just bumps a version key today). Run it
      // alongside a short minimum spin window so the click always produces
      // perceptible feedback instead of an instant, invisible no-op.
      await Promise.all([
        Promise.resolve(onRefresh()),
        new Promise(resolve => setTimeout(resolve, 600)),
      ]);
    } catch (err) {
      console.error('[ui-flow][memory-controls] manual refresh failed', err);
    } finally {
      setRefreshing(false);
      console.debug('[ui-flow][memory-controls] manual refresh: exit');
    }
  }, [onRefresh, refreshing]);

  return (
    <div className="flex flex-wrap items-center justify-between gap-3" data-testid="memory-actions">
      <ModeToggle mode={mode} onChange={onModeChange} />

      <div className="flex flex-wrap items-center gap-2">
        {/* Destructive actions — muted, set apart behind a divider. */}
        <Button
          variant="secondary"
          tone="danger"
          size="sm"
          onClick={handleWipe}
          disabled={busy}
          data-testid="memory-wipe-all"
          className="text-content-muted border-line"
          leadingIcon={wiping ? <Spinner /> : <TrashIcon />}
          title={t('workspace.wipeTitle')}>
          {wiping ? t('workspace.resetting') : t('workspace.resetMemory')}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={handleResetTree}
          disabled={busy}
          data-testid="memory-reset-tree"
          className="text-content-muted border-line hover:border-amber-300 hover:bg-amber-50 hover:text-amber-700 dark:hover:border-amber-500/30 dark:hover:bg-amber-500/10 dark:hover:text-amber-300"
          leadingIcon={resetting ? <Spinner /> : <RefreshIcon />}
          title={t('workspace.resetTreeTitle')}>
          {resetting ? t('workspace.rebuilding') : t('workspace.resetMemoryTree')}
        </Button>

        <Separator orientation="vertical" className="mx-1 h-5 self-center bg-surface-strong" />

        {/* Secondary actions — quiet ghost buttons. */}
        <Button
          variant="secondary"
          size="sm"
          onClick={handleRefresh}
          disabled={refreshing}
          aria-busy={refreshing}
          data-testid="memory-graph-refresh"
          leadingIcon={refreshing ? <Spinner /> : <RefreshIcon />}
          title={t('common.refresh')}>
          {t('common.refresh')}
        </Button>
        {contentRootAbs ? (
          <ObsidianVaultSection contentRootAbs={contentRootAbs} onToast={onToast} />
        ) : null}

        {/* Primary action. */}
        <Button
          variant="primary"
          size="sm"
          onClick={handleBuildTrees}
          disabled={building}
          data-testid="memory-build-trees"
          leadingIcon={building ? <Spinner /> : <BrainIcon />}>
          {building ? t('workspace.building') : t('workspace.buildSummaryTrees')}
        </Button>
      </div>
    </div>
  );
}

// ── Mode toggle ───────────────────────────────────────────────────────────────

interface ModeToggleProps {
  mode: GraphMode;
  onChange: (next: GraphMode) => void;
}

function ModeToggle({ mode, onChange }: ModeToggleProps) {
  const { t } = useT();
  return (
    <ChipTabs<GraphMode>
      items={[
        { id: 'tree', label: t('workspace.trees'), testId: 'memory-graph-mode-tree' },
        { id: 'contacts', label: t('workspace.contacts'), testId: 'memory-graph-mode-contacts' },
      ]}
      value={mode}
      onChange={onChange}
      as="tab"
      ariaLabel={t('workspace.graphViewMode')}
      testId="memory-graph-mode-toggle"
      className="inline-flex items-center gap-1.5"
    />
  );
}

// ── Tiny inline icons (no extra dep) ──────────────────────────────────────────

function RefreshIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M21 12a9 9 0 11-3-6.7" />
      <path d="M21 4v5h-5" />
      <path d="M3 12a9 9 0 003 6.7" />
      <path d="M3 20v-5h5" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M3 6h18" />
      <path d="M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2" />
      <path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </svg>
  );
}

function BrainIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M9.5 2A2.5 2.5 0 0112 4.5v15a2.5 2.5 0 01-4.96.44 2.5 2.5 0 01-2.96-3.08 3 3 0 01-.34-5.58 2.5 2.5 0 011.32-4.24 2.5 2.5 0 011.98-3A2.5 2.5 0 019.5 2z" />
      <path d="M14.5 2A2.5 2.5 0 0012 4.5v15a2.5 2.5 0 004.96.44 2.5 2.5 0 002.96-3.08 3 3 0 00.34-5.58 2.5 2.5 0 00-1.32-4.24 2.5 2.5 0 00-1.98-3A2.5 2.5 0 0014.5 2z" />
    </svg>
  );
}

function Spinner() {
  return (
    <svg
      className="animate-spin"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden="true">
      <circle cx="12" cy="12" r="9" opacity="0.25" />
      <path d="M21 12a9 9 0 00-9-9" />
    </svg>
  );
}
