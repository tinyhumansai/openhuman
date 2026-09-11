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
import { useCallback, useEffect, useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import { Button, ModalShell, TextField } from '../ui';

type Request = { refId: string; keyName: string; prompt: string };

function SecretPromptDialog() {
  const { t } = useT();
  const titleId = useId();
  const inputId = useId();
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
    <ModalShell
      onClose={handleCancel}
      title={t('mcp.setup.secretDialog.title')}
      titleId={titleId}
      closePolicy={{ escape: !submitting, backdrop: !submitting, button: !submitting }}
      contentClassName="px-0 py-0"
      footer={
        <div className="flex items-center justify-end gap-3">
          <Button variant="tertiary" onClick={handleCancel} disabled={submitting}>
            {t('mcp.setup.secretDialog.cancel')}
          </Button>
          <Button
            type="submit"
            form="mcp-setup-secret-form"
            disabled={submitting || value.length === 0}>
            {submitting
              ? t('mcp.setup.secretDialog.submitting')
              : t('mcp.setup.secretDialog.submit')}
          </Button>
        </div>
      }>
      <form id="mcp-setup-secret-form" onSubmit={handleSubmit}>
        <div className="px-6 pt-2 pb-4">
          <p className="text-sm text-content-secondary">
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
            htmlFor={inputId}
            className="block text-xs font-medium text-content-secondary mb-1">
            {t('mcp.setup.secretDialog.inputLabel')}
          </label>
          <div className="flex items-stretch gap-2">
            <TextField
              id={inputId}
              type={reveal ? 'text' : 'password'}
              autoComplete="off"
              autoCorrect="off"
              spellCheck={false}
              value={value}
              onChange={e => setValue(e.target.value)}
              placeholder={t('mcp.setup.secretDialog.inputPlaceholder')}
              mono
              className="flex-1"
              autoFocus
              disabled={submitting}
            />
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => setReveal(v => !v)}
              disabled={submitting}>
              {reveal ? t('mcp.setup.secretDialog.hide') : t('mcp.setup.secretDialog.show')}
            </Button>
          </div>
          <p className="text-[11px] text-content-muted mt-2">
            {t('mcp.setup.secretDialog.privacyNote')}
          </p>
          {error && (
            <p className="text-xs text-coral-500 mt-2">
              {t('mcp.setup.secretDialog.errorPrefix')} {error}
            </p>
          )}
        </div>
      </form>
    </ModalShell>
  );
}

export default SecretPromptDialog;
