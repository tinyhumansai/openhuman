import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  type Backend,
  capabilityForModel,
  DEFAULT_EXTRACT_MODEL,
  downloadAsset,
  fetchInstalledModels,
  getMemoryTreeLlm,
  type ModelDescriptor,
  REQUIRED_EMBEDDER_MODEL,
  setMemoryTreeLlm,
} from '../../lib/intelligence/settingsApi';
import BackendChooser from './BackendChooser';
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
  // Single Memory LLM that drives both extractor and summariser. Most
  // users want one model for both; the rare case of mixing them is not
  // worth the second dropdown's cognitive cost.
  const [memoryModel, setMemoryModel] = useState<string>(DEFAULT_EXTRACT_MODEL);
  const [installedModels, setInstalledModels] = useState<string[]>([]);

  // One-shot bootstrap — pull current backend and the installed-model list.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        console.debug('[intelligence-settings] bootstrap');
        const [bk, models] = await Promise.all([getMemoryTreeLlm(), fetchInstalledModels()]);
        if (cancelled) return;
        setBackend(bk);
        setInstalledModels(models.map(m => m.name));
      } catch (err) {
        if (!cancelled) {
          // Bootstrap failure leaves the tab on its useState defaults
          // (cloud backend, empty installed list) rather than throwing
          // an unhandled rejection. The user can still flip the backend
          // chooser; subsequent reads will retry the RPCs.
          console.error('[intelligence-settings] bootstrap failed', err);
        }
      }
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
    } catch (err) {
      console.error('[intelligence-settings] backend switch failed', err);
    } finally {
      setBackendBusy(false);
    }
  }, []);

  // Persist Memory LLM changes to config.toml. Fans out to both
  // extractor and summariser keys in a single atomic write — the unified
  // UI is one dropdown, but the underlying schema retains both keys so
  // power users can still split them via the RPC directly if needed.
  const handleMemoryModelChange = useCallback(
    async (id: string) => {
      console.debug('[intelligence-settings] memory model -> %s', id);
      const previous = memoryModel;
      setMemoryModel(id);
      try {
        await setMemoryTreeLlm('local', { extractModel: id, summariserModel: id });
      } catch (err) {
        // Persistence failed → roll back the optimistic UI update so the
        // dropdown reflects the value that's actually saved on disk
        // rather than the one the user just attempted.
        setMemoryModel(previous);
        console.error('[intelligence-settings] persist memory model failed', err);
      }
    },
    [memoryModel]
  );

  const handleDownload = useCallback(async (model: ModelDescriptor) => {
    const cap = capabilityForModel(model);
    if (!cap) {
      console.debug('[intelligence-settings] no capability for model', { id: model.id });
      return;
    }
    try {
      await downloadAsset(cap);
    } catch (err) {
      console.error('[intelligence-settings] model download failed', err);
    } finally {
      // Refresh installed list after any download attempt — even on
      // failure, Ollama may have partially landed assets we should
      // surface; if it hasn't, the next bootstrap tick will catch up.
      const refreshed = await fetchInstalledModels();
      setInstalledModels(refreshed.map(m => m.name));
    }
  }, []);

  const handleUse = useCallback(
    (model: ModelDescriptor) => {
      if (model.roles.includes('extract') || model.roles.includes('summariser')) {
        void handleMemoryModelChange(model.id);
      }
    },
    [handleMemoryModelChange]
  );

  const activeModelIds = useMemo<string[]>(() => {
    const ids = new Set<string>();
    ids.add(memoryModel);
    ids.add(REQUIRED_EMBEDDER_MODEL);
    return [...ids];
  }, [memoryModel]);

  return (
    <div className="space-y-10" data-testid="intelligence-settings-tab">
      <Section title="AI backend">
        <BackendChooser value={backend} onChange={handleBackendChange} busy={backendBusy} />
      </Section>

      {/* All local-model sections (assignment, catalog, currently-loaded)
          are gated on local backend. Cloud users get just the backend
          chooser + the explanatory copy that lives inside it — they don't
          need to see Ollama-related UI at all. */}
      {backend === 'local' && (
        <>
          <Section title="Model assignment">
            <ModelAssignment
              installedModelIds={installedModels}
              memoryModel={memoryModel}
              onChangeMemory={handleMemoryModelChange}
            />
          </Section>

          <Section title="Model catalog">
            <ModelCatalog
              installedModelIds={installedModels}
              activeModelIds={activeModelIds}
              onDownload={handleDownload}
              onUse={handleUse}
            />
          </Section>
        </>
      )}
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
