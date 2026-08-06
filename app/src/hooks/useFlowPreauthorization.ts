import debug from 'debug';
import { useCallback, useState } from 'react';

import { preauthorizeFlow } from '../services/api/approvalApi';
import {
  type ApprovalManifest,
  getApprovalManifest,
  setFlowEnabled,
} from '../services/api/flowsApi';

const log = debug('openhuman:flows:preauthorization');

/** Whether the manifest warrants the consolidated card at all: missing
 * grants to approve, or tier-Blocked rows the user must see at save time
 * (a Block only surfaces as a failed run otherwise — the autonomy model
 * treats Block as its own gate decision, never a silent success path). */
const needsCard = (manifest: ApprovalManifest): boolean =>
  manifest.missing.length > 0 || manifest.entries.some(entry => entry.kind === 'blocked');

/** The consolidated save+enable card waiting on a user decision. */
export interface FlowPreauthorizationPending {
  flowId: string;
  manifest: ApprovalManifest;
  /** Whether "Approve all" should also enable the flow afterwards. */
  enableOnApprove: boolean;
}

export type FlowPreauthorizationOutcome =
  /** Nothing to ask — the flow proceeded without the card. */
  | 'no-card'
  /** All missing grants approved (and the flow enabled when requested). */
  | 'approved'
  /** User denied — the flow was left/turned off. */
  | 'denied';

/**
 * Orchestrates the consolidated flow pre-authorization card ("Approve all" /
 * "Deny") shown when a flow is saved and enabled:
 *
 * - `beginEnable(flowId)` — the enable path (list-row toggle, "Save & enable").
 *   Fetches the approval manifest; enables directly when nothing is missing,
 *   otherwise surfaces the card and defers the enable to "Approve all".
 * - `checkAfterSave(flowId, isEnabled)` — the canvas save path, where the flow
 *   may already be enabled. Surfaces the card only when grants are missing;
 *   "Deny" then turns the flow off ("saved but not live").
 * - `approveAll()` / `deny()` — resolve the pending card. Approve batches the
 *   grants via `openhuman.approval_preauthorize_flow`, then enables. Deny
 *   ensures the flow is disabled.
 *
 * Fail-open on manifest errors: a broken manifest RPC must never make a flow
 * impossible to enable, so `beginEnable` falls back to a plain enable (the
 * runtime ApprovalGate still parks per-node exactly as before this feature).
 */
export function useFlowPreauthorization(opts?: {
  onSettled?: (outcome: FlowPreauthorizationOutcome, flowId: string) => void;
}) {
  const [pending, setPending] = useState<FlowPreauthorizationPending | null>(null);
  const [busy, setBusy] = useState(false);
  const [errorKey, setErrorKey] = useState<string | null>(null);
  const onSettled = opts?.onSettled;

  const beginEnable = useCallback(
    async (flowId: string): Promise<boolean> => {
      log('beginEnable: id=%s', flowId);
      let manifest: ApprovalManifest | null = null;
      try {
        manifest = await getApprovalManifest({ id: flowId });
      } catch (err) {
        log('beginEnable: manifest fetch failed, enabling without card: %o', err);
      }
      if (!manifest || !manifest.gate_installed || !needsCard(manifest)) {
        await setFlowEnabled(flowId, true);
        log('beginEnable: enabled without card (missing=%d)', manifest?.missing.length ?? -1);
        onSettled?.('no-card', flowId);
        return true;
      }
      log(
        'beginEnable: %d grant(s) missing, blocked=%s — showing card',
        manifest.missing.length,
        manifest.entries.some(entry => entry.kind === 'blocked')
      );
      setErrorKey(null);
      setPending({ flowId, manifest, enableOnApprove: true });
      return false;
    },
    [onSettled]
  );

  const checkAfterSave = useCallback(
    async (flowId: string, isEnabled: boolean): Promise<boolean> => {
      log('checkAfterSave: id=%s enabled=%s', flowId, isEnabled);
      if (!isEnabled) return false;
      let manifest: ApprovalManifest;
      try {
        manifest = await getApprovalManifest({ id: flowId });
      } catch (err) {
        log('checkAfterSave: manifest fetch failed — skipping card: %o', err);
        return false;
      }
      if (!manifest.gate_installed || !needsCard(manifest)) {
        onSettled?.('no-card', flowId);
        return false;
      }
      log(
        'checkAfterSave: %d grant(s) missing, blocked=%s — showing card',
        manifest.missing.length,
        manifest.entries.some(entry => entry.kind === 'blocked')
      );
      setErrorKey(null);
      // Already enabled: approve keeps it on; deny turns it off.
      setPending({ flowId, manifest, enableOnApprove: false });
      return true;
    },
    [onSettled]
  );

  const approveAll = useCallback(async () => {
    if (!pending || busy) return;
    setBusy(true);
    setErrorKey(null);
    try {
      // A blocked-only card has nothing to grant — skip the RPC and just
      // proceed to the enable ("Enable anyway"): the tier gate keeps
      // blocking at runtime, the card's job was the save-time warning.
      if (pending.manifest.missing.length > 0) {
        const result = await preauthorizeFlow(pending.flowId, pending.manifest.missing);
        log(
          'approveAll: id=%s granted=%d already=%d',
          pending.flowId,
          result.granted.length,
          result.already_trusted.length
        );
      } else {
        log('approveAll: id=%s nothing approvable (blocked-only card)', pending.flowId);
      }
      if (pending.enableOnApprove) {
        await setFlowEnabled(pending.flowId, true);
      }
      setPending(null);
      onSettled?.('approved', pending.flowId);
    } catch (err) {
      log('approveAll failed: id=%s err=%o', pending.flowId, err);
      setErrorKey('flows.enableApproval.error');
    } finally {
      setBusy(false);
    }
  }, [pending, busy, onSettled]);

  const deny = useCallback(async () => {
    if (!pending || busy) return;
    setBusy(true);
    setErrorKey(null);
    try {
      // "Deny" = the flow is saved but must not be live. Idempotent when the
      // flow was never enabled.
      await setFlowEnabled(pending.flowId, false);
      log('deny: id=%s left disabled', pending.flowId);
      setPending(null);
      onSettled?.('denied', pending.flowId);
    } catch (err) {
      log('deny failed: id=%s err=%o', pending.flowId, err);
      setErrorKey('flows.enableApproval.error');
    } finally {
      setBusy(false);
    }
  }, [pending, busy, onSettled]);

  return { pending, busy, errorKey, beginEnable, checkAfterSave, approveAll, deny };
}
