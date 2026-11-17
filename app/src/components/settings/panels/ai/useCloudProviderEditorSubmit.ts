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
        ca_cert_pem: p.caCertPem ?? '',
        auth_style: p.authStyle,
      }));

      // Live verification — flush the new cloud_providers list and call
      // `/models` through the Rust controller. Flush first so Rust validates
      // the CA bundle before a new or replacement credential is persisted.
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
            ca_cert_pem: p.caCertPem ?? '',
            auth_style: p.authStyle,
          }));
        try {
          await flushCloudProviders(nextWireProviders);
          // The provider must be configured before probing, and its CA has now
          // passed Rust-side validation. Let key-write failures surface inline.
          if (apiKey) {
            await setCloudProviderKey(upserted.slug, apiKey);
          }
        } catch (writeError) {
          // A flush may have applied before a transport error, so restore the
          // previous list for either a failed flush or a failed key write.
          await flushCloudProviders(priorWireProviders).catch(rollbackErr =>
            console.warn(
              `[ai-settings] rollback flush after provider write failure slug=${upserted.slug}`,
              rollbackErr
            )
          );
          throw writeError;
        }
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
