/**
 * useFlowValidation (Phase 3c) — debounced + on-demand validation of the live
 * editable-canvas draft against `openhuman.flows_validate`.
 *
 * The canvas serializes its controlled node/edge state to a `WorkflowGraph` on
 * every edit; this hook watches that graph (keyed by its serialized form so a
 * no-op re-render never re-validates) and, `DEBOUNCE_MS` after the last change,
 * asks the core to validate it. It also exposes {@link FlowValidationState.validateNow}
 * for the explicit "Validate" button (immediate, bypasses the debounce).
 *
 * Failures of the RPC itself (offline, no Tauri bridge, transport error) are
 * swallowed — validation is advisory client-side (the server re-validates on
 * `flows_update` before persisting), so a transport hiccup must never wedge the
 * editor with a stale "invalid" state or an unhandled rejection. On failure the
 * last successful `validation` is left untouched and `validating` clears.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import type { WorkflowGraph } from '../../../lib/flows/types';
import { type FlowValidation, validateFlow } from '../../../services/api/flowsApi';

const log = createDebug('app:flows:canvas:validate');

/** Idle delay after the last edit before auto-validating the draft. */
export const VALIDATION_DEBOUNCE_MS = 500;

export interface FlowValidationState {
  /** The most recent successful validation result, or `null` before the first. */
  validation: FlowValidation | null;
  /** True while a validation RPC is in flight (debounced or manual). */
  validating: boolean;
  /** Validate the current graph immediately, resolving with the result (or `null` on error). */
  validateNow: () => Promise<FlowValidation | null>;
}

/**
 * @param graph    the live draft graph (read via a ref, so its identity changing
 *                 every render doesn't itself trigger re-validation).
 * @param graphKey a stable serialization of `graph`; the debounced effect keys
 *                 off this so validation only re-runs when the graph truly changes.
 * @param enabled  gate the auto-validate effect (e.g. off in read-only hosts).
 */
export function useFlowValidation(
  graph: WorkflowGraph,
  graphKey: string,
  enabled = true
): FlowValidationState {
  const [validation, setValidation] = useState<FlowValidation | null>(null);
  const [validating, setValidating] = useState(false);

  // Latest graph, read lazily so the debounced effect / manual trigger always
  // validate the current draft without listing `graph` (new object each render)
  // as a dependency.
  const graphRef = useRef(graph);
  graphRef.current = graph;

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Monotonic request token: guards against an out-of-order completion (a
  // debounced auto-validate and a manual `validateNow()`, or two rapid
  // `validateNow()` calls, resolving out of issue order) applying a stale
  // result over a fresher one — which could wrongly unblock or re-block Save
  // after the user has made further edits. Also doubles as the correlation
  // field for the debug log.
  const requestIdRef = useRef(0);

  const run = useCallback(async (): Promise<FlowValidation | null> => {
    const requestId = ++requestIdRef.current;
    setValidating(true);
    try {
      const result = await validateFlow(graphRef.current);
      if (!mountedRef.current || requestId !== requestIdRef.current) return result;
      log(
        'validated (req=%d): valid=%s errors=%d warnings=%d',
        requestId,
        result.valid,
        result.errors.length,
        result.warnings.length
      );
      setValidation(result);
      return result;
    } catch (err) {
      log('validation failed (non-fatal): %o', err);
      return null;
    } finally {
      if (mountedRef.current && requestId === requestIdRef.current) setValidating(false);
    }
  }, []);

  // Debounced auto-validate: re-runs only when the serialized graph changes.
  useEffect(() => {
    if (!enabled) return;
    const timer = setTimeout(() => {
      void run();
    }, VALIDATION_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [graphKey, enabled, run]);

  return { validation, validating, validateNow: run };
}

export default useFlowValidation;
