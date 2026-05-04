import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  type Backend,
  capabilityForModel,
  DEFAULT_EXTRACT_MODEL,
  DEFAULT_SUMMARISER_MODEL,
  downloadAsset,
  fetchInstalledModels,
  fetchLocalAiStatus,
  getMemoryTreeLlm,
  type ModelDescriptor,
  REQUIRED_EMBEDDER_MODEL,
  setMemoryTreeLlm,
} from '../../lib/intelligence/settingsApi';
import type { LocalAiStatus } from '../../utils/tauriCommands';
import BackendChooser from './BackendChooser';
import CurrentlyLoaded from './CurrentlyLoaded';
import ModelAssignment from './ModelAssignment';
import ModelCatalog from './ModelCatalog';

/**
 * Settings tab for the Intelligence page.
 *
 * Layout (top → bottom):
 *   1. AI Backend         — Cloud / Local toggle
 *   2. Model Assignment   — per-role dropdowns (visible only in Local mode)
 *   3. Model Catalog      — full curated list with download / use / delete
 *   4. Currently Loaded   — live `/api/ps`-style readout
 *
 * The orchestrator owns the cross-section state (backend, role assignments,
 * cached installed-models / status). Sections themselves stay presentational.
 */
export default function IntelligenceSettingsTab() {
  const [backend, setBackend] = useState<Backend>('cloud');
  const [backendBusy, setBackendBusy] = useState(false);
  const [extractModel, setExtractModel] = useState<string>(DEFAULT_EXTRACT_MODEL);
  const [summariserModel, setSummariserModel] = useState<string>(DEFAULT_SUMMARISER_MODEL);
  const [installedModels, setInstalledModels] = useState<string[]>([]);
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus | null>(null);

  // One-shot bootstrap — pull current backend and the installed-model list.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      console.debug('[intelligence-settings] bootstrap');
      const [bk, models, status] = await Promise.all([
        getMemoryTreeLlm(),
        fetchInstalledModels(),
        fetchLocalAiStatus(),
      ]);
      if (cancelled) return;
      setBackend(bk);
      setInstalledModels(models.map(m => m.name));
      setLocalAiStatus(status);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleBackendChange = useCallback(async (next: Backend) => {
    setBackendBusy(true);
    try {
      const { effective } = await setMemoryTreeLlm(next);
      setBackend(effective);
    } finally {
      setBackendBusy(false);
    }
  }, []);

  // Persist Extract dropdown changes to config.toml in the same atomic
  // write the backend toggle uses. We pass `backend: 'local'` (the only
  // mode where ModelAssignment is rendered) plus the single role being
  // changed — the absent fields keep the other models untouched, so two
  // dropdown changes back-to-back compose cleanly without fighting over
  // disk writes. UI state is updated optimistically pre-RPC; if the call
  // throws we still want the dropdown to reflect the user's pick rather
  // than snap back, matching BackendChooser's silent-on-success pattern.
  const handleExtractModelChange = useCallback(async (id: string) => {
    console.debug('[intelligence-settings] extract model -> %s', id);
    setExtractModel(id);
    try {
      await setMemoryTreeLlm('local', { extractModel: id });
    } catch (err) {
      console.error('[intelligence-settings] persist extract model failed', err);
    }
  }, []);

  const handleSummariserModelChange = useCallback(async (id: string) => {
    console.debug('[intelligence-settings] summariser model -> %s', id);
    setSummariserModel(id);
    try {
      await setMemoryTreeLlm('local', { summariserModel: id });
    } catch (err) {
      console.error('[intelligence-settings] persist summariser model failed', err);
    }
  }, []);

  const handleDownload = useCallback(async (model: ModelDescriptor) => {
    const cap = capabilityForModel(model);
    if (!cap) {
      console.debug('[intelligence-settings] no capability for model', { id: model.id });
      return;
    }
    await downloadAsset(cap);
    // Refresh installed list after a download attempt; if Ollama hasn't
    // surfaced the new model yet the next bootstrap tick will catch it.
    const refreshed = await fetchInstalledModels();
    setInstalledModels(refreshed.map(m => m.name));
  }, []);

  const handleUse = useCallback((model: ModelDescriptor) => {
    if (model.roles.includes('summariser')) {
      setSummariserModel(model.id);
      return;
    }
    if (model.roles.includes('extract')) {
      setExtractModel(model.id);
    }
  }, []);

  const activeModelIds = useMemo<string[]>(() => {
    const ids = new Set<string>();
    ids.add(extractModel);
    ids.add(summariserModel);
    ids.add(REQUIRED_EMBEDDER_MODEL);
    return [...ids];
  }, [extractModel, summariserModel]);

  return (
    <div className="space-y-10" data-testid="intelligence-settings-tab">
      <Section title="AI backend">
        <BackendChooser value={backend} onChange={handleBackendChange} busy={backendBusy} />
      </Section>

      {backend === 'local' && (
        <Section title="Model assignment">
          <ModelAssignment
            installedModelIds={installedModels}
            extractModel={extractModel}
            summariserModel={summariserModel}
            onChangeExtract={handleExtractModelChange}
            onChangeSummariser={handleSummariserModelChange}
          />
        </Section>
      )}

      <Section title="Model catalog">
        <ModelCatalog
          installedModelIds={installedModels}
          activeModelIds={activeModelIds}
          onDownload={handleDownload}
          onUse={handleUse}
        />
      </Section>

      <Section title="Currently loaded">
        <CurrentlyLoaded
          status={localAiStatus}
          backend={backend}
          installedModelIds={installedModels}
        />
      </Section>
    </div>
  );
}

interface SectionProps {
  title: string;
  children: React.ReactNode;
}

function Section({ title, children }: SectionProps) {
  return (
    <section>
      <h2 className="font-display text-[11px] uppercase tracking-[0.18em] text-stone-400 mb-3">
        {title}
      </h2>
      {children}
    </section>
  );
}
