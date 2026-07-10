import debug from 'debug';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { authorize, listConnections } from '../../lib/composio/composioApi';
import { canonicalizeComposioToolkitSlug } from '../../lib/composio/toolkitSlug';
import { deriveComposioState } from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import { clearPendingApprovalForThread, type PendingApproval } from '../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../store/hooks';
import { openUrl } from '../../utils/openUrl';
import {
  getRequiredFieldsForToolkit,
  validateRequiredFieldValues,
} from '../composio/toolkitRequiredFields';
import Button from '../ui/Button';

/**
 * Inline OAuth connect card (#3993).
 *
 * Rendered in place of {@link ApprovalRequestCard} when the agent calls the
 * `composio_connect` tool — that tool parks on the same ApprovalGate, so the
 * request arrives over the identical `approval_request` socket path, but the
 * surface is a **Connect** button rather than approve/deny. Clicking it runs
 * `composio_authorize`, opens the OAuth handoff in the browser, and polls
 * `composio_list_connections` until the toolkit flips to ACTIVE — at which
 * point it resolves the parked tool call with `approve_once` so the agent
 * resumes in the same turn. Cancel (or the gate's 10-minute TTL) resolves it
 * as `deny`.
 *
 * Provider-specific required fields (WhatsApp `waba_id`, Jira `subdomain`,
 * Dynamics 365 `org_name`) are collected inline from the
 * [`toolkitRequiredFields`] registry before the OAuth handoff — so even
 * field-gated toolkits connect entirely in-chat rather than failing with a
 * raw `ConnectedAccount_MissingRequiredFields` (code 612) error. Mirrors the
 * polling + field-collection contract of `ComposioConnectModal` so the two
 * connect surfaces behave identically.
 */
const log = debug('openhuman:chat:integration-connect-card');

const POLL_INTERVAL_MS = 4_000;
const POLL_TIMEOUT_MS = 5 * 60 * 1_000;

/**
 * Composio error slug for missing required fields (code 612). Defensive
 * recovery path: if the backend starts requiring a field the registry hasn't
 * caught up on, surface a clear message instead of a dead retry loop.
 */
const MISSING_REQUIRED_FIELDS_SLUG = 'ConnectedAccount_MissingRequiredFields';

type Phase = 'idle' | 'connecting' | 'error';

interface Props {
  threadId: string;
  approval: PendingApproval;
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export const IntegrationConnectCard: React.FC<Props> = ({ threadId, approval }) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  // Canonicalize the slug the agent supplied (e.g. `google_drive` →
  // `googledrive`) so authorize / list-connections hit the form Composio's
  // backend expects (#3993). Defensive: the core already canonicalizes too.
  const toolkit = canonicalizeComposioToolkitSlug(approval.toolkit ?? '');

  const [phase, setPhase] = useState<Phase>('idle');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  // Cleared to false on a permanent backend rejection (no auth config, unknown
  // toolkit, 400) so the card drops its Retry affordance — retrying won't help.
  const [retryable, setRetryable] = useState(true);

  // Provider-specific required fields are sourced from the declarative
  // registry — no per-toolkit branches here (mirrors ComposioConnectModal).
  const requiredFields = useMemo(() => getRequiredFieldsForToolkit(toolkit), [toolkit]);
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const pollTimerRef = useRef<number | null>(null);
  const pollDeadlineRef = useRef<number>(0);
  const isPollingRef = useRef<boolean>(false);
  const inFlightRef = useRef<boolean>(false);
  // Set once the card is dismissed (Deny) or unmounted, so an `authorize()`
  // call still in flight doesn't open OAuth / start polling afterwards.
  const cancelledRef = useRef<boolean>(false);

  const stopPolling = useCallback(() => {
    isPollingRef.current = false;
    if (pollTimerRef.current != null) {
      window.clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  }, []);

  // Stop polling if the card unmounts (turn ended, thread switched, decided),
  // and mark cancelled so any in-flight authorize continuation aborts.
  useEffect(
    () => () => {
      cancelledRef.current = true;
      stopPolling();
    },
    [stopPolling]
  );

  // Resolve the parked `composio_connect` tool call. `approve_once` once the
  // connection is live; `deny` when the user cancels. Clears the card on a
  // successful decide — ChatRuntimeProvider also clears on turn end.
  const resolveGate = useCallback(
    async (decision: 'approve_once' | 'deny') => {
      try {
        await callCoreRpc({
          method: 'openhuman.approval_decide',
          params: { request_id: approval.requestId, decision },
        });
      } catch (e) {
        // The backend request is still parked. Clearing the card here would
        // drop the only surface that can retry/deny it, blocking the thread
        // until the gate TTL expires — so keep the card mounted and surface
        // the failure instead of clearing it (#4062, coderabbit review).
        log('approval_decide(%s) failed: %o', decision, e);
        setPhase('error');
        setErrorMsg(t('chat.approval.error'));
        return;
      }
      dispatch(clearPendingApprovalForThread({ threadId }));
    },
    [approval.requestId, dispatch, threadId, t]
  );

  const startPolling = useCallback(() => {
    stopPolling();
    isPollingRef.current = true;
    pollDeadlineRef.current = Date.now() + POLL_TIMEOUT_MS;

    const scheduleNext = () => {
      if (!isPollingRef.current) return;
      pollTimerRef.current = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
    };

    const tick = async () => {
      if (inFlightRef.current || !isPollingRef.current) return;
      if (Date.now() > pollDeadlineRef.current) {
        stopPolling();
        setPhase('error');
        setErrorMsg(t('composio.connect.oauthTimeout'));
        // Resolve the parked tool call now so the agent resumes immediately
        // instead of blocking until the 10-min gate TTL (#3993). The agent
        // relays the timeout and the user can ask to connect again.
        await resolveGate('deny');
        return;
      }
      inFlightRef.current = true;
      try {
        const resp = await listConnections();
        // Scan ALL rows for this toolkit — list_connections returns every row
        // (failed / pending / multiple accounts), so the freshly-authorized
        // ACTIVE row can sit behind an older FAILED or pending one (#3993,
        // codex review). Approve if any row is connected.
        const matches = resp.connections.filter(
          c => c.toolkit.toLowerCase() === toolkit.toLowerCase()
        );
        if (matches.some(c => deriveComposioState(c) === 'connected')) {
          stopPolling();
          await resolveGate('approve_once');
          return;
        }
        // Keep waiting while any handoff is still in flight; only surface an
        // error once a failed row exists and nothing is pending.
        const pending = matches.some(c => deriveComposioState(c) === 'pending');
        const errored = matches.find(c => deriveComposioState(c) === 'error');
        if (errored && !pending) {
          stopPolling();
          setPhase('error');
          setErrorMsg(
            t('composio.connect.connectionFailed').replace('{status}', String(errored.status))
          );
          return;
        }
      } catch (err) {
        // Transient poll failures are expected mid-handoff — retry next tick.
        log('connection poll failed: %o', err);
      } finally {
        inFlightRef.current = false;
      }
      scheduleNext();
    };

    void tick();
  }, [resolveGate, stopPolling, t, toolkit]);

  const connect = useCallback(async () => {
    if (phase === 'connecting' || !toolkit) return;
    // A prior Deny (or a failed decide that kept the card mounted) may have set
    // cancelledRef — clear it so this fresh, user-initiated attempt isn't
    // aborted by the post-authorize cancellation guard.
    cancelledRef.current = false;

    // Collect + validate provider-specific required fields before the OAuth
    // handoff so field-gated toolkits don't hit a 612 error mid-flow.
    let extraParams: Record<string, string> | undefined;
    if (requiredFields.length > 0) {
      const errors = validateRequiredFieldValues(requiredFields, fieldValues);
      if (Object.keys(errors).length > 0) {
        setFieldErrors(errors);
        return;
      }
      setFieldErrors({});
      extraParams = {};
      for (const f of requiredFields) {
        extraParams[f.key] = (fieldValues[f.key] ?? '').trim();
      }
    }

    setPhase('connecting');
    setErrorMsg(null);
    setRetryable(true);
    try {
      const resp = await authorize(toolkit, extraParams);
      // The user may have hit Deny / dismissed the card while authorize was in
      // flight — abort so we don't open OAuth or start polling after the gate
      // was already denied, nor race a second approval_decide (codex review).
      if (cancelledRef.current) return;
      try {
        await openUrl(resp.connectUrl);
      } catch (openErr) {
        // Opening the browser failed, but the handoff may still be reachable;
        // keep polling and let the user reopen if needed.
        log('openUrl failed: %o', openErr);
      }
      startPolling();
    } catch (e) {
      log('authorize failed: %o', e);
      setPhase('error');
      // Defensive: backend reports a required field the registry lacks. Without
      // a field definition we can't collect it inline, so surface a clear
      // "needs extra setup" message rather than a retry that will fail again.
      if (errorText(e).includes(MISSING_REQUIRED_FIELDS_SLUG) && requiredFields.length === 0) {
        setErrorMsg(t('composio.connect.additionalConfigRequired'));
      } else {
        // Surface the backend's actual reason (e.g. "toolkit not in allowlist")
        // so a failed connect is diagnosable — a bare "Connection failed." hides
        // whether the toolkit is unsupported, mis-slugged, or a transient error.
        // Composio authorize errors are connection diagnostics (no PII); bound
        // the length and collapse whitespace before showing.
        // Strip the "(status: {status})" clause locale-robustly — the English
        // literal differs per locale (de "Status", fr "statut", …), so match the
        // parenthetical containing the {status} token rather than the English
        // wording, else non-English users see a literal "{status}" (#3993).
        const base = t('composio.connect.connectionFailed')
          .replace(/\s*\([^)]*\{status\}[^)]*\)/, '')
          .trim();
        const reason = errorText(e).replace(/\s+/g, ' ').trim().slice(0, 240);
        setErrorMsg(reason ? `${base} ${reason}` : base);
        // Permanent backend rejections won't change on retry — drop the Retry
        // affordance so the user isn't looped on a doomed connect (#3993).
        if (/no auth config|not a valid toolkit|unknown toolkit|not found|\b400\b/i.test(reason)) {
          setRetryable(false);
        }
      }
    }
  }, [phase, requiredFields, fieldValues, startPolling, t, toolkit]);

  const cancel = useCallback(async () => {
    cancelledRef.current = true;
    stopPolling();
    await resolveGate('deny');
  }, [resolveGate, stopPolling]);

  const connecting = phase === 'connecting';
  const showFields = requiredFields.length > 0 && !connecting;

  return (
    <div
      role="group"
      aria-label={approval.message || t('composio.connect.connect')}
      className="rounded-xl border border-primary-200 bg-primary-50 p-3.5 text-sm shadow-sm dark:border-primary-800 dark:bg-primary-950">
      <div className="flex items-start gap-3">
        <span
          aria-hidden
          className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary-100 text-sm text-primary-600 dark:bg-primary-900 dark:text-primary-300">
          🔗
        </span>
        <div className="min-w-0 flex-1">
          <p className="break-words font-semibold text-content">
            {approval.message || t('chat.approval.fallback')}
          </p>

          {showFields && (
            <div className="mt-2.5 flex flex-col gap-2.5">
              {requiredFields.map(field => (
                <label key={field.key} className="block text-xs text-content-secondary">
                  <span className="font-medium">{t(field.labelKey)}</span>
                  <span className="mt-1 flex items-center gap-1.5">
                    <input
                      type="text"
                      value={fieldValues[field.key] ?? ''}
                      placeholder={field.placeholderKey ? t(field.placeholderKey) : undefined}
                      onChange={e =>
                        setFieldValues(prev => ({ ...prev, [field.key]: e.target.value }))
                      }
                      className="min-w-0 flex-1 rounded-lg border border-line-strong bg-neutral-0 px-2.5 py-1.5 text-ink outline-none transition focus:border-primary-500 focus:ring-1 focus:ring-primary-500 dark:bg-surface dark:text-content"
                    />
                    {field.suffix && (
                      <span className="shrink-0 text-content-faint">{field.suffix}</span>
                    )}
                  </span>
                  {field.hintKey && (
                    <span className="mt-1 block text-content-muted">{t(field.hintKey)}</span>
                  )}
                  {fieldErrors[field.key] && (
                    <span className="mt-1 block text-coral-600 dark:text-coral-400">
                      {t(fieldErrors[field.key])}
                    </span>
                  )}
                </label>
              ))}
            </div>
          )}

          {connecting && (
            <p className="mt-1.5 flex items-center gap-1.5 text-xs text-primary-700 dark:text-primary-300">
              <span aria-hidden className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary-500" />
              {t('composio.connect.waitingHint')}
            </p>
          )}

          <p className="mt-1.5 text-xs text-content-faint">
            {t('chat.approval.tool')}{' '}
            <span className="font-mono text-content-muted">{approval.toolName}</span>
          </p>

          {errorMsg && (
            <p className="mt-2 text-xs text-coral-600 dark:text-coral-400">⚠ {errorMsg}</p>
          )}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            {/* Hide Connect/Retry on a permanent rejection — retrying a toolkit
                the backend can't authorize just loops. Dismiss stays. */}
            {!(phase === 'error' && !retryable) && (
              <Button
                variant="primary"
                size="sm"
                data-analytics-id="chat-integration-connect"
                onClick={() => void connect()}
                disabled={connecting || !toolkit}>
                {connecting
                  ? t('chat.approval.deciding')
                  : phase === 'error'
                    ? t('composio.connect.retryConnection')
                    : t('composio.connect.connect')}
              </Button>
            )}
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="chat-integration-connect-cancel"
              onClick={() => void cancel()}>
              {t('chat.approval.deny')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default IntegrationConnectCard;
