// Listens for `openhuman:mcp-setup-secret-requested` window events dispatched
// by `socketService` and renders a native input dialog so the user can hand
// the core a secret value out-of-band.
//
// The dialog deliberately uses `<input type="password">` so the value isn't
// echoed in the UI by default and never lands in clipboard history via
// triple-click. On submit, the value is POSTed straight to
// `openhuman.mcp_setup_submit_secret` and immediately cleared from React
// state — no logging, no Redux, no persistence on this side. The MCP setup
// agent only sees the opaque `ref://<hex>` ref returned by
// `mcp_setup_request_secret`; the raw value never enters the LLM context.
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';

type Request = { refId: string; keyName: string; prompt: string };

export function SecretPromptDialog() {
  const { t } = useT();
  const [request, setRequest] = useState<Request | null>(null);
  const [value, setValue] = useState('');
  const [reveal, setReveal] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onRequest = (event: Event) => {
      const detail = (event as CustomEvent).detail as Request | undefined;
      if (!detail?.refId || !detail.keyName) return;
      setRequest(detail);
      setValue('');
      setReveal(false);
      setError(null);
      setSubmitting(false);
    };
    window.addEventListener('openhuman:mcp-setup-secret-requested', onRequest);
    return () => {
      window.removeEventListener('openhuman:mcp-setup-secret-requested', onRequest);
    };
  }, []);

  const reset = useCallback(() => {
    setRequest(null);
    setValue('');
    setReveal(false);
    setError(null);
    setSubmitting(false);
  }, []);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!request || submitting || value.length === 0) return;
      setSubmitting(true);
      setError(null);
      try {
        await callCoreRpc({
          method: 'openhuman.mcp_setup_submit_secret',
          params: { ref_id: request.refId, value },
        });
        // Wipe local state on success — the value has now moved into the
        // core's process-local SETUP_SECRETS map; React doesn't need a
        // copy. Closing the dialog also drops the React-tree reference.
        reset();
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
        setSubmitting(false);
      }
    },
    [request, submitting, value, reset]
  );

  // Cancel: do NOT call mcp_setup_submit_secret. The agent-side
  // `request_secret` will hit its 5-minute timeout and return an error
  // the agent can surface to the user, which is the right outcome here.
  const handleCancel = useCallback(() => {
    if (submitting) return;
    reset();
  }, [submitting, reset]);

  if (!request) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/40 animate-fade-in"
      onClick={handleCancel}
      role="dialog"
      aria-modal="true"
      aria-label={t('mcp.setup.secretDialog.title')}>
      <div
        className="bg-surface rounded-2xl max-w-md w-full shadow-large border border-line animate-slide-up"
        onClick={e => e.stopPropagation()}>
        <form onSubmit={handleSubmit}>
          <div className="p-6 pb-4">
            <h2 className="text-lg font-semibold text-content">
              {t('mcp.setup.secretDialog.title')}
            </h2>
            <p className="text-sm text-content-secondary mt-2">
              {t('mcp.setup.secretDialog.bodyPrefix')}{' '}
              <code className="px-1.5 py-0.5 rounded bg-surface-subtle text-content font-mono text-xs">
                {request.keyName}
              </code>
              {t('mcp.setup.secretDialog.bodySuffix')}
            </p>
            {request.prompt && (
              <p className="text-sm text-content-secondary mt-3 whitespace-pre-wrap">
                {request.prompt}
              </p>
            )}
          </div>

          <div className="px-6 pb-2">
            <label
              htmlFor="mcp-setup-secret-input"
              className="block text-xs font-medium text-content-secondary mb-1">
              {t('mcp.setup.secretDialog.inputLabel')}
            </label>
            <div className="flex items-stretch gap-2">
              <input
                id="mcp-setup-secret-input"
                type={reveal ? 'text' : 'password'}
                autoComplete="off"
                autoCorrect="off"
                spellCheck={false}
                value={value}
                onChange={e => setValue(e.target.value)}
                placeholder={t('mcp.setup.secretDialog.inputPlaceholder')}
                className="flex-1 px-3 py-2 rounded-lg border border-line-strong bg-surface-muted text-content font-mono text-sm focus:outline-none focus:ring-2 focus:ring-primary-500"
                autoFocus
                disabled={submitting}
              />
              <button
                type="button"
                onClick={() => setReveal(v => !v)}
                disabled={submitting}
                className="px-3 py-2 text-xs font-medium text-content-secondary rounded-lg border border-line-strong hover:bg-surface-hover">
                {reveal ? t('mcp.setup.secretDialog.hide') : t('mcp.setup.secretDialog.show')}
              </button>
            </div>
            <p className="text-[11px] text-content-muted dark:text-content-faint mt-2">
              {t('mcp.setup.secretDialog.privacyNote')}
            </p>
            {error && (
              <p className="text-xs text-coral-500 mt-2">
                {t('mcp.setup.secretDialog.errorPrefix')} {error}
              </p>
            )}
          </div>

          <div className="flex items-center justify-end gap-3 p-6 pt-4 border-t border-line">
            <button
              type="button"
              onClick={handleCancel}
              disabled={submitting}
              className="px-4 py-2 text-sm font-medium text-content-secondary hover:text-content rounded-lg hover:bg-surface-hover disabled:opacity-50">
              {t('mcp.setup.secretDialog.cancel')}
            </button>
            <button
              type="submit"
              disabled={submitting || value.length === 0}
              className="px-4 py-2 text-sm font-medium rounded-lg bg-primary-500 hover:bg-primary-600 text-content-inverted disabled:opacity-50">
              {submitting
                ? t('mcp.setup.secretDialog.submitting')
                : t('mcp.setup.secretDialog.submit')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default SecretPromptDialog;
