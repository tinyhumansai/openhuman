import { KeyRound } from 'lucide-react';
import { useEffect, useState } from 'react';

import { useKeyringOperationApprovals } from '../../hooks/useKeyringOperationApprovals';
import { useT } from '../../lib/i18n/I18nContext';
import { respondToKeyringOperation } from '../../services/keyringApi';
import Alert from '../ui/Alert';
import Button from '../ui/Button';
import { ModalShell } from '../ui/ModalShell';

const ACCESS_LABEL_KEYS = {
  read: 'keyring.operation.access.read',
  write: 'keyring.operation.access.write',
  delete: 'keyring.operation.access.delete',
} as const;

const KeyringOperationApprovalOverlay = () => {
  const { requests, dismiss } = useKeyringOperationApprovals();
  const { t } = useT();
  const request = requests[0];
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const requestId = request?.request_id;

  useEffect(() => {
    if (!requestId) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [requestId]);

  if (!request) return null;

  const secondsRemaining = Math.max(0, Math.ceil((request.expires_at_ms - now) / 1_000));
  const decide = async (approved: boolean) => {
    setBusy(true);
    setError(false);
    try {
      await respondToKeyringOperation(request.request_id, approved);
      dismiss(request.request_id);
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  };

  return (
    <ModalShell
      title={t('keyring.operation.title')}
      titleId="keyring-operation-approval-title"
      subtitle={t('keyring.operation.description')}
      icon={<KeyRound className="h-4 w-4" aria-hidden />}
      onClose={() => undefined}
      closePolicy={{ escape: false, backdrop: false, button: false }}
      testId="keyring-operation-approval-dialog"
      maxWidthClassName="max-w-lg">
      <div className="space-y-4">
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 text-sm">
          <dt className="text-content-muted">{t('keyring.operation.access')}</dt>
          <dd className="font-medium text-content">{t(ACCESS_LABEL_KEYS[request.access])}</dd>
          <dt className="text-content-muted">{t('keyring.operation.requester')}</dt>
          <dd className="break-words text-content">{request.caller_agent}</dd>
          <dt className="text-content-muted">{t('keyring.operation.origin')}</dt>
          <dd className="break-words text-content">{request.origin}</dd>
          <dt className="text-content-muted">{t('keyring.operation.purpose')}</dt>
          <dd className="break-words text-content">{request.purpose}</dd>
          <dt className="text-content-muted">{t('keyring.operation.name')}</dt>
          <dd className="break-all font-mono text-xs text-content">{request.name}</dd>
        </dl>

        <p className="text-xs text-content-muted">
          {t('keyring.operation.expires').replace('{seconds}', String(secondsRemaining))}
        </p>

        {error && (
          <Alert variant="destructive" density="compact" role="alert">
            {t('keyring.operation.error')}
          </Alert>
        )}

        <div className="flex justify-end gap-2">
          <Button
            variant="secondary"
            tone="danger"
            disabled={busy}
            onClick={() => void decide(false)}>
            {busy ? t('keyring.operation.submitting') : t('keyring.operation.deny')}
          </Button>
          <Button disabled={busy || secondsRemaining === 0} onClick={() => void decide(true)}>
            {busy ? t('keyring.operation.submitting') : t('keyring.operation.allow')}
          </Button>
        </div>
      </div>
    </ModalShell>
  );
};

export default KeyringOperationApprovalOverlay;
