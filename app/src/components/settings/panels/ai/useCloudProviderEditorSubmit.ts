/*
 * Submit handler for the advanced `CloudProviderEditor` modal — live-verifies
 * the new/edited provider (unless the user explicitly skipped verification)
 * and rolls back on failure.
 */
import { useCallback } from 'react';

import {
  clearCloudProviderKey,
  flushCloudProviders,
  listProviderModels,
  setCloudProviderKey,
} from '../../../../services/api/aiSettingsApi';
import { presentProviderSetupError } from '../ProviderSetupErrorNotice';
import {
  type AISettings,
  type CloudProvider,
  maskKeyLabel,
  ProviderProbeError,
} from './aiPanelTypes';

export function useCloudProviderEditorSubmit({
  editing,
  draft,
  saved,
  persist,
  t,
  onDone,
}: {
  editing: CloudProvider | 'new' | null;
  draft: AISettings;
  saved: AISettings;
  persist: (next: AISettings) => Promise<void>;
  t: (key: string, fallback?: string) => string;
  onDone: () => void;
}) {
  return useCallback(
    async (next: CloudProvider, apiKey: string, opts?: { skipProbe?: boolean }) => {
      const id =
        editing === 'new' || !editing?.id
          ? `p_${next.slug}_${Math.random().toString(36).slice(2, 7)}`
          : editing.id;
      const upserted: CloudProvider = {
        ...next,
        id,
        maskedKey: maskKeyLabel(apiKey ? true : next.maskedKey.startsWith('••••')),
      };

      // Snapshot the prior persisted cloud_providers list so we can restore
      // it if the live probe fails.
      const priorWireProviders = saved.cloudProviders.map(p => ({
        id: p.id,
        slug: p.slug,
        label: p.label,
        endpoint: p.endpoint,
        auth_style: p.authStyle,
      }));

      // Persist the credential BEFORE the probe so the factory has it
      // available. Let setCloudProviderKey throw — the editor's button-click
      // handler catches and surfaces the error inline.
      if (apiKey && upserted.slug !== 'openhuman') {
        await setCloudProviderKey(upserted.slug, apiKey);
      }

      // Live verification — flush the new cloud_providers list and call
      // `/models` through the Rust controller. Skip for the OpenHuman backend
      // (session JWT, no probe-able endpoint).
      if (upserted.slug !== 'openhuman') {
        const list =
          editing === 'new' || !editing
            ? [...draft.cloudProviders, upserted]
            : draft.cloudProviders.map(p => (p.id === editing.id ? upserted : p));
        const nextWireProviders = list
          .filter(p => !['', 'cloud', 'openhuman', 'pid'].includes(p.slug))
          .map(p => ({
            id: p.id,
            slug: p.slug,
            label: p.label,
            endpoint: p.endpoint,
            auth_style: p.authStyle,
          }));
        await flushCloudProviders(nextWireProviders);
        // `skipProbe` is the user's explicit "add it anyway" after a failed
        // verification. A provider whose `/models` listing is absent or
        // auth-shaped differently (Azure's classic `api-version` surface is
        // both) is still perfectly usable for inference (#5213).
        if (!opts?.skipProbe) {
          try {
            await listProviderModels(upserted.slug);
          } catch (probeErr) {
            // Roll back both stores. Failures are LOGGED, never swallowed:
            // a silently failed key-clear orphans the key on disk (#5339).
            await flushCloudProviders(priorWireProviders).catch(rollbackErr =>
              console.warn(`[ai-settings] rollback flush failed slug=${upserted.slug}`, rollbackErr)
            );
            if (apiKey) {
              await clearCloudProviderKey(upserted.slug).catch(rollbackErr =>
                console.warn(
                  `[ai-settings] rollback clearCloudProviderKey failed slug=${upserted.slug}`,
                  rollbackErr
                )
              );
            }
            const msg = probeErr instanceof Error ? probeErr.message : String(probeErr);
            console.warn('[ai-settings] provider /models probe failed', {
              slug: upserted.slug,
              summary: presentProviderSetupError(msg, t).summary,
            });
            throw new ProviderProbeError(`Could not reach ${upserted.label}: ${msg}`);
          }
        }
      }

      const list =
        editing === 'new' || !editing
          ? [...draft.cloudProviders, upserted]
          : draft.cloudProviders.map(p => (p.id === editing.id ? upserted : p));
      await persist({ ...draft, cloudProviders: list });
      onDone();
    },
    [editing, draft, saved.cloudProviders, persist, t, onDone]
  );
}
