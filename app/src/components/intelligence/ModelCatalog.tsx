import { useState } from 'react';

import {
  capabilityForModel,
  type ModelDescriptor,
  RECOMMENDED_MODEL_CATALOG,
} from '../../lib/intelligence/settingsApi';

interface ModelCatalogProps {
  /** Names of models that are already installed on the user's machine. */
  installedModelIds: ReadonlyArray<string>;
  /** Models in active use right now (assigned to a role). */
  activeModelIds: ReadonlyArray<string>;
  /** Called when the user kicks off a download for a catalog entry. */
  onDownload: (model: ModelDescriptor) => Promise<void>;
  /** Called when the user wants to assign an installed model to its role. */
  onUse: (model: ModelDescriptor) => void;
  /** Called when the user removes an installed model. */
  onDelete?: (model: ModelDescriptor) => Promise<void>;
}

type RowState = 'idle' | 'downloading' | 'error';

/**
 * Single-column list of curated models. Each row is one card showing
 *   <id>   <size>   <status>   [action]
 * The action button changes by state:
 *   - not installed → "Download" (clicks fire the per-capability RPC)
 *   - installed but unused → "Use"
 *   - installed and active → "Active"
 *   - downloading → inline progress bar (mocked client-side animation
 *     since the per-asset RPC is fire-and-forget; the real progress
 *     stream is wired in `local_ai_downloads_progress` polling — out of
 *     scope for v1)
 */
// Ollama reports tags as `<name>:<tag>` (e.g. `bge-m3:latest`,
// `gemma3:1b-it-qat`). The recommended catalog uses bare names for the
// default-`:latest` case (e.g. `bge-m3`) and full `<name>:<tag>` for
// non-default tags. Normalize both sides by stripping the `:latest`
// suffix before comparing — that way `bge-m3` matches `bge-m3:latest`,
// while `gemma3:1b-it-qat` still requires the explicit tag.
function normalizeModelId(id: string): string {
  return id.endsWith(':latest') ? id.slice(0, -':latest'.length) : id;
}

export default function ModelCatalog({
  installedModelIds,
  activeModelIds,
  onDownload,
  onUse,
  onDelete,
}: ModelCatalogProps) {
  const installedSet = new Set(installedModelIds.map(normalizeModelId));
  const activeSet = new Set(activeModelIds.map(normalizeModelId));

  return (
    <div className="space-y-2">
      {RECOMMENDED_MODEL_CATALOG.map(model => (
        <CatalogRow
          key={model.id}
          model={model}
          installed={installedSet.has(normalizeModelId(model.id))}
          active={activeSet.has(normalizeModelId(model.id))}
          onDownload={onDownload}
          onUse={onUse}
          onDelete={onDelete}
        />
      ))}
    </div>
  );
}

interface CatalogRowProps {
  model: ModelDescriptor;
  installed: boolean;
  active: boolean;
  onDownload: ModelCatalogProps['onDownload'];
  onUse: ModelCatalogProps['onUse'];
  onDelete: ModelCatalogProps['onDelete'];
}

function CatalogRow({ model, installed, active, onDownload, onUse, onDelete }: CatalogRowProps) {
  const [state, setState] = useState<RowState>('idle');
  const [progress, setProgress] = useState(0);

  const status: 'active' | 'installed' | 'available' = active
    ? 'active'
    : installed
      ? 'installed'
      : 'available';

  const handleDownload = async () => {
    setState('downloading');
    setProgress(8);
    // Animated mock progress while the real per-capability RPC is in flight.
    // The real download progress stream comes from
    // `openhumanLocalAiDownloadsProgress` polling — wiring that in is
    // tracked separately and out of scope for v1.
    const tick = setInterval(() => {
      setProgress(prev => {
        if (prev >= 90) return prev;
        return prev + Math.max(2, Math.round((100 - prev) * 0.06));
      });
    }, 220);
    let didFail = false;
    try {
      await onDownload(model);
      setProgress(100);
    } catch (err) {
      console.debug('[intelligence-settings] catalog download failed', { id: model.id, err });
      setState('error');
      didFail = true;
    } finally {
      clearInterval(tick);
      // Hold the terminal state long enough for the user to actually read
      // it. Success collapses fast (~600 ms) so the row settles back to
      // its post-install state without a long pause; error lingers ~3s
      // so an unsuccessful pull doesn't snap back before the user has
      // a chance to notice. Tracked via a local flag because `state` is
      // React state and won't reflect the just-issued `setState('error')`
      // until the next render.
      const settleMs = didFail ? 3000 : 600;
      window.setTimeout(() => {
        setState('idle');
        setProgress(0);
      }, settleMs);
    }
  };

  return (
    <div className="border border-stone-200 rounded-xl bg-white px-4 py-3">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-3 min-w-0 flex-1">
          <div className="text-sm font-medium text-stone-900 truncate">{model.id}</div>
          <div className="font-mono text-[11px] text-stone-500 whitespace-nowrap">{model.size}</div>
          <StatusChip status={status} />
        </div>
        <div className="flex items-center gap-2">
          {state === 'downloading' ? (
            <ProgressBar progress={progress} />
          ) : (
            <ActionButton
              status={status}
              hasDelete={!!onDelete}
              onDownload={handleDownload}
              onUse={() => onUse(model)}
              onDelete={onDelete ? () => onDelete(model) : undefined}
            />
          )}
        </div>
      </div>
      <div className="mt-1 flex items-center gap-3 font-mono text-[11px] text-stone-500">
        <span>{model.ramHint}</span>
        <span>·</span>
        <span>{model.category}</span>
        <span>·</span>
        <span className="text-stone-400">{model.note}</span>
        {capabilityForModel(model) === null && (
          <span className="text-amber-600 ml-auto">no capability binding</span>
        )}
      </div>
      {state === 'error' && (
        <div className="mt-2 text-[11px] text-coral-700">
          Download failed — check Ollama is running and try again.
        </div>
      )}
    </div>
  );
}

function StatusChip({ status }: { status: 'active' | 'installed' | 'available' }) {
  if (status === 'active') {
    return (
      <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] uppercase tracking-wider rounded-full bg-sage-50 text-sage-700 border border-sage-100">
        active
      </span>
    );
  }
  if (status === 'installed') {
    return (
      <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] uppercase tracking-wider rounded-full bg-stone-100 text-stone-600 border border-stone-200">
        installed
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] uppercase tracking-wider rounded-full bg-white text-stone-500 border border-stone-200">
      not downloaded
    </span>
  );
}

interface ActionButtonProps {
  status: 'active' | 'installed' | 'available';
  hasDelete: boolean;
  onDownload: () => void;
  onUse: () => void;
  onDelete?: () => void;
}

function ActionButton({ status, hasDelete, onDownload, onUse, onDelete }: ActionButtonProps) {
  if (status === 'active') {
    return (
      <span className="px-3 py-1.5 text-xs text-stone-500 border border-transparent">in use</span>
    );
  }
  if (status === 'installed') {
    return (
      <div className="flex items-center gap-1.5">
        <button
          type="button"
          onClick={onUse}
          className="px-3 py-1.5 text-xs font-medium bg-primary-50 hover:bg-primary-100 text-primary-700 border border-primary-100 rounded-lg transition-colors">
          Use
        </button>
        {hasDelete && onDelete && (
          <button
            type="button"
            onClick={onDelete}
            className="px-2 py-1.5 text-xs text-stone-500 hover:text-coral-700 border border-stone-200 rounded-lg transition-colors"
            aria-label="Delete model">
            Delete
          </button>
        )}
      </div>
    );
  }
  return (
    <button
      type="button"
      onClick={onDownload}
      className="px-3 py-1.5 text-xs font-medium bg-white hover:bg-stone-50 text-stone-700 border border-stone-200 rounded-lg transition-colors">
      Download
    </button>
  );
}

function ProgressBar({ progress }: { progress: number }) {
  return (
    <div
      className="w-32 h-2 rounded-full bg-stone-100 overflow-hidden"
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(progress)}>
      <div
        className="h-full bg-primary-500 transition-all duration-200"
        style={{ width: `${Math.min(100, Math.max(0, progress))}%` }}
      />
    </div>
  );
}
