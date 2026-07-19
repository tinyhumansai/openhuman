/**
 * `useCreateFlow` (Phase 4a/4c) — shared create-and-open logic for the
 * new-workflow chooser, the template gallery, and the Workflows empty state.
 * Persists a candidate `WorkflowGraph` via `flows_create` and, on success,
 * navigates into the editable canvas at `/flows/:id`. Single-flight: a second
 * call while one is in flight is ignored, so a double-click can't create two
 * flows.
 *
 * `busyKey` identifies which affordance is mid-create (a template id, or
 * `'blank'` for start-from-scratch) so a caller can show the spinner on just
 * that card/button. On failure the key clears and `error` is set to the
 * localized `flows.chooser.createError` message, leaving the surface open to
 * retry.
 */
import createDebug from 'debug';
import { useCallback, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import type { WorkflowGraph } from '../../lib/flows/types';
import { useT } from '../../lib/i18n/I18nContext';
import { createFlow } from '../../services/api/flowsApi';

const log = createDebug('app:flows:create');

/** Sentinel `busyKey` for the "start from scratch" path (not a template id). */
export const BLANK_FLOW_KEY = 'blank';

export interface UseCreateFlow {
  /** Persist `graph` under `name`, then navigate into its canvas. `key` tags the busy affordance. */
  create: (key: string, name: string, graph: WorkflowGraph) => Promise<void>;
  /** The `key` of the create currently in flight, or `null`. */
  busyKey: string | null;
  /** Localized create-failure message, or `null`. */
  error: string | null;
  /** Clear the error banner (e.g. when the user switches views). */
  clearError: () => void;
}

export function useCreateFlow(): UseCreateFlow {
  const navigate = useNavigate();
  const { t } = useT();
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const create = useCallback(
    async (key: string, name: string, graph: WorkflowGraph) => {
      if (busyKey) {
        log('create: ignored — already creating key=%s', busyKey);
        return;
      }
      log('create: key=%s name=%s nodes=%d', key, name, graph.nodes.length);
      setBusyKey(key);
      setError(null);
      try {
        const flow = await createFlow(name, graph);
        log('create: created id=%s — navigating to canvas', flow.id);
        navigate(`/flows/${flow.id}`);
      } catch (err) {
        log('create: failed key=%s err=%o', key, err);
        setError(t('flows.chooser.createError'));
        setBusyKey(null);
      }
    },
    [busyKey, navigate, t]
  );

  const clearError = useCallback(() => setError(null), []);

  return { create, busyKey, error, clearError };
}
