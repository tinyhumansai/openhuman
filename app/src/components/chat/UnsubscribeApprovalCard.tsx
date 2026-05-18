import React, { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import Button from '../ui/Button';

interface UnsubscribePayload {
  status: string;
  action: string;
  metadata: { sender: string; unsubscribe_link: string; message: string };
}

interface Props {
  payload: UnsubscribePayload;
}

export const UnsubscribeApprovalCard: React.FC<Props> = ({ payload }) => {
  const { t } = useT();
  const [status, setStatus] = useState<'pending' | 'approved' | 'denied'>('pending');
  const [isProcessing, setIsProcessing] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  useEffect(() => {
    setStatus('pending');
    setIsProcessing(false);
    setErrorMsg(null);
  }, [payload]);

  const handleApprove = async () => {
    if (isProcessing || status === 'approved') return;
    setIsProcessing(true);
    setErrorMsg(null);
    try {
      // Typically, you would call a core RPC method to execute the URL/mailto
      // or instruct the agent to proceed.
      await callCoreRpc({
        method: 'tools::execute_unsubscribe',
        params: { link: payload.metadata.unsubscribe_link },
      });
      setStatus('approved');
    } catch (e: any) {
      console.error('Unsubscribe failed', e);
      setStatus('pending');
      setErrorMsg(e?.message || 'Missing permissions or network error');
    } finally {
      setIsProcessing(false);
    }
  };

  const handleDeny = () => {
    setStatus('denied');
    setErrorMsg(null);
    // Optionally notify the agent of the denial so it can update its context
  };

  if (payload.action !== 'unsubscribe' || payload.status !== 'pending_approval') return null;

  return (
    <div className="border border-gray-200 dark:border-gray-800 rounded-lg p-4 my-2 bg-gray-50 dark:bg-gray-900">
      <div className="flex items-start gap-3">
        <div className="text-xl">📧</div>
        <div className="flex-1">
          <h4 className="font-semibold text-sm text-gray-900 dark:text-gray-100">
            {t('chat.unsubscribeApproval.title')}
          </h4>
          <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">
            {payload.metadata.message}
          </p>
          <div className="text-xs text-gray-500 mt-2 font-mono break-all bg-gray-100 dark:bg-gray-800 p-2 rounded">
            {payload.metadata.unsubscribe_link}
          </div>

          {errorMsg && (
            <div className="text-sm text-red-600 font-medium mt-2 bg-red-50 dark:bg-red-900/20 p-2 rounded">
              ⚠️ {errorMsg}
            </div>
          )}

          {status === 'pending' && (
            <div className="flex gap-2 mt-4">
              <Button variant="primary" size="sm" onClick={handleApprove} disabled={isProcessing}>
                {isProcessing
                  ? t('chat.unsubscribeApproval.processing')
                  : t('chat.unsubscribeApproval.approve')}
              </Button>
              <Button variant="secondary" size="sm" onClick={handleDeny} disabled={isProcessing}>
                {t('chat.unsubscribeApproval.deny')}
              </Button>
            </div>
          )}

          {status === 'approved' && (
            <div className="text-sm text-green-600 font-medium mt-3">
              {t('chat.unsubscribeApproval.approved')}
            </div>
          )}
          {status === 'denied' && (
            <div className="text-sm text-red-600 font-medium mt-3">
              {t('chat.unsubscribeApproval.denied')}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
