/**
 * Security banner — surfaces the host-aware approval-gate boot state to the
 * user. Reads `openhuman.approval_get_gate_state` on mount and renders one of
 * two banners:
 *
 * - Persistent red banner when `disabledByEnv === true` — operator set
 *   `OPENHUMAN_APPROVAL_GATE=0` on a standalone host (CLI / Docker) and the
 *   gate is actually OFF. External-effect tool calls will run unprompted.
 *
 * - One-shot yellow info banner when `overrideIgnored === true` — the same
 *   env override was attempted under the Tauri desktop shell, which always
 *   rejects it. The user sees that their attempt was blocked, but the gate
 *   is still on. Auto-dismisses after 10 s.
 *
 * Otherwise renders nothing — the steady-state desktop boot path is silent.
 *
 * Mounted near the top of the provider chain in `App.tsx` so it surfaces
 * regardless of route.
 */
import { useEffect, useState } from 'react';

import { useT } from '../lib/i18n/I18nContext';
import { type ApprovalGateBootState, fetchApprovalGateState } from '../services/api/approvalApi';

const OVERRIDE_IGNORED_AUTO_DISMISS_MS = 10_000;

interface SecurityBannerProps {
  /** Override the fetcher for tests; defaults to the live RPC client. */
  fetchState?: () => Promise<ApprovalGateBootState>;
}

const SecurityBanner = ({ fetchState = fetchApprovalGateState }: SecurityBannerProps) => {
  const { t } = useT();
  const [state, setState] = useState<ApprovalGateBootState | null>(null);
  const [dismissedIgnored, setDismissedIgnored] = useState(false);

  useEffect(() => {
    let cancelled = false;
    fetchState()
      .then(s => {
        if (!cancelled) setState(s);
      })
      .catch(() => {
        // Silently ignore — the fetcher already returns a benign default
        // on RPC failure. A degraded core must never blank the app shell.
      });
    return () => {
      cancelled = true;
    };
  }, [fetchState]);

  // Auto-dismiss the override-ignored info banner after the timeout — it's a
  // one-shot acknowledgement, not a persistent warning. The persistent red
  // banner for `disabledByEnv` does NOT auto-dismiss; the user has to act
  // (restart with the env unset) to clear it.
  useEffect(() => {
    if (!state?.overrideIgnored) return;
    const id = window.setTimeout(() => setDismissedIgnored(true), OVERRIDE_IGNORED_AUTO_DISMISS_MS);
    return () => {
      window.clearTimeout(id);
    };
  }, [state?.overrideIgnored]);

  if (!state) return null;

  // Order matters: when both flags are somehow true (shouldn't happen given
  // the Rust decision tree is mutually exclusive, but defend in depth), the
  // persistent disabled banner wins — it's the higher-severity message.
  if (state.disabledByEnv) {
    return (
      <div
        role="alert"
        aria-live="polite"
        data-testid="security-banner-gate-disabled"
        style={{
          position: 'fixed',
          top: 0,
          left: 0,
          right: 0,
          zIndex: 9000,
          padding: '10px 16px',
          background: '#7f1d1d',
          color: '#fef2f2',
          fontFamily: 'Inter, system-ui, sans-serif',
          fontSize: 13,
          fontWeight: 500,
          textAlign: 'center',
          borderBottom: '1px solid #b91c1c',
          boxShadow: '0 1px 2px rgba(0,0,0,0.15)',
        }}>
        <strong style={{ marginRight: 8 }}>{t('security.approvalGateDisabled.title')}</strong>
        <span style={{ opacity: 0.92 }}>{t('security.approvalGateDisabled.body')}</span>
      </div>
    );
  }

  if (state.overrideIgnored && !dismissedIgnored) {
    return (
      <div
        role="status"
        aria-live="polite"
        data-testid="security-banner-override-ignored"
        style={{
          position: 'fixed',
          top: 0,
          left: 0,
          right: 0,
          zIndex: 9000,
          padding: '8px 16px',
          background: '#78350f',
          color: '#fffbeb',
          fontFamily: 'Inter, system-ui, sans-serif',
          fontSize: 12,
          fontWeight: 500,
          textAlign: 'center',
          borderBottom: '1px solid #b45309',
        }}>
        <strong style={{ marginRight: 8 }}>
          {t('security.approvalGateOverrideIgnored.title')}
        </strong>
        <span style={{ opacity: 0.92 }}>{t('security.approvalGateOverrideIgnored.body')}</span>
      </div>
    );
  }

  return null;
};

export default SecurityBanner;
