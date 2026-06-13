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

// ── Shared button system ──────────────────────────────────────────────────────

const BTN_BASE =
  'inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus:ring-2';
const BTN_PRIMARY = `${BTN_BASE} bg-primary-500 text-white shadow-sm hover:bg-primary-600 focus:ring-primary-200`;
const BTN_GHOST = `${BTN_BASE} border border-stone-200 bg-white text-stone-700 shadow-sm hover:bg-stone-50 focus:ring-stone-200 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200 dark:hover:bg-neutral-800 dark:focus:ring-neutral-700`;
// Destructive actions read as proper (bordered) buttons but stay muted until
// hover, when they reveal their warning tint.
const BTN_MUTED = `${BTN_BASE} border border-stone-200 bg-white text-stone-500 shadow-sm focus:ring-stone-200 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-400 dark:focus:ring-neutral-700`;

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
  const busy = building || wiping || resetting;

  const handleWipe = useCallback(async () => {
    // Two-step confirm so accidental clicks can't nuke a workspace.
    if (!window.confirm(t('workspace.wipeConfirm'))) return;
    setWiping(true);
    try {
      const resp = await memoryTreeWipeAll();
      onToast?.({
        type: 'success',
        title: 'Memory wiped',
        message:
          `Removed ${resp.rows_deleted.toLocaleString()} row(s) and ` +
          `${resp.dirs_removed.length} folder(s); cleared ` +
          `${resp.sync_state_cleared.toLocaleString()} sync-state cursor(s). ` +
          `Click Sync on a connected source to repopulate.`,
      });
      // Re-pull immediately so the canvas reflects the wipe.
      onRefresh();
    } catch (err) {
      console.error('[ui-flow][memory-controls] wipe_all failed', err);
      onToast?.({
        type: 'error',
        title: 'Reset failed',
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
        title: 'Memory tree rebuilding',
        message:
          `Cleared ${resp.tree_rows_deleted.toLocaleString()} tree row(s); ` +
          `requeued ${resp.chunks_requeued.toLocaleString()} chunk(s) ` +
          `(${resp.jobs_enqueued.toLocaleString()} extract jobs). ` +
          `The graph will fill back in as the worker drains.`,
      });
      // reset_tree restarts from extract jobs (slower than seal-only) — give the
      // worker a longer head start than build does before re-pulling.
      setTimeout(() => onRefresh(), 8000);
    } catch (err) {
      console.error('[ui-flow][memory-controls] reset_tree failed', err);
      onToast?.({
        type: 'error',
        title: 'Could not reset memory tree',
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
        title: 'Could not build summary trees',
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setBuilding(false);
    }
  }, [onToast, onRefresh]);

  return (
    <div className="flex flex-wrap items-center justify-between gap-3" data-testid="memory-actions">
      <ModeToggle mode={mode} onChange={onModeChange} />

      <div className="flex flex-wrap items-center gap-2">
        {/* Destructive actions — muted, set apart behind a divider. */}
        <button
          type="button"
          onClick={handleWipe}
          disabled={busy}
          data-testid="memory-wipe-all"
          className={`${BTN_MUTED} hover:border-coral-300 hover:bg-coral-50 hover:text-coral-600 dark:hover:border-coral-500/30 dark:hover:bg-coral-500/10 dark:hover:text-coral-300`}
          title={t('workspace.wipeTitle')}>
          {wiping ? <Spinner /> : <TrashIcon />}
          {wiping ? t('workspace.resetting') : t('workspace.resetMemory')}
        </button>
        <button
          type="button"
          onClick={handleResetTree}
          disabled={busy}
          data-testid="memory-reset-tree"
          className={`${BTN_MUTED} hover:border-amber-300 hover:bg-amber-50 hover:text-amber-700 dark:hover:border-amber-500/30 dark:hover:bg-amber-500/10 dark:hover:text-amber-300`}
          title={t('workspace.resetTreeTitle')}>
          {resetting ? <Spinner /> : <RefreshIcon />}
          {resetting ? t('workspace.rebuilding') : t('workspace.resetMemoryTree')}
        </button>

        <span aria-hidden className="mx-1 h-5 w-px self-center bg-stone-200 dark:bg-neutral-700" />

        {/* Secondary actions — quiet ghost buttons. */}
        <button
          type="button"
          onClick={onRefresh}
          data-testid="memory-graph-refresh"
          className={BTN_GHOST}
          title={t('common.refresh')}>
          <RefreshIcon /> {t('common.refresh')}
        </button>
        {contentRootAbs ? (
          <ObsidianVaultSection contentRootAbs={contentRootAbs} onToast={onToast} />
        ) : null}

        {/* Primary action. */}
        <button
          type="button"
          onClick={handleBuildTrees}
          disabled={building}
          data-testid="memory-build-trees"
          className={BTN_PRIMARY}>
          {building ? <Spinner /> : <BrainIcon />}
          {building ? t('workspace.building') : t('workspace.buildSummaryTrees')}
        </button>
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
  const baseBtn =
    'px-3.5 py-1.5 text-xs font-semibold rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary-200';
  const active = 'bg-primary-500 text-white shadow-sm';
  const idle =
    'text-stone-600 hover:text-stone-900 dark:text-neutral-300 dark:hover:text-neutral-100';
  return (
    <div
      className="inline-flex items-center gap-1 rounded-full border border-stone-200 bg-stone-50 p-1 dark:border-neutral-800 dark:bg-neutral-800/60"
      role="tablist"
      aria-label={t('workspace.graphViewMode')}
      data-testid="memory-graph-mode-toggle">
      <button
        type="button"
        onClick={() => onChange('tree')}
        className={`${baseBtn} ${mode === 'tree' ? active : idle}`}
        role="tab"
        aria-selected={mode === 'tree'}
        data-testid="memory-graph-mode-tree">
        {t('workspace.trees')}
      </button>
      <button
        type="button"
        onClick={() => onChange('contacts')}
        className={`${baseBtn} ${mode === 'contacts' ? active : idle}`}
        role="tab"
        aria-selected={mode === 'contacts'}
        data-testid="memory-graph-mode-contacts">
        {t('workspace.contacts')}
      </button>
    </div>
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
