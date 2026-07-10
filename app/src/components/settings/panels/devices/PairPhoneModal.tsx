/**
 * PairPhoneModal
 *
 * Opens a pairing session via `devices_create_pairing`, shows a QR code the
 * iPhone user scans, then polls `devices_list` every 2 s to detect when the
 * device has completed the handshake (DevicePaired).  Handles expiry and lets
 * the user regenerate the code.
 *
 * TODO(future): replace the 2-second poll with a real socket event bridge when
 * the Rust core forwards DomainEvent::DevicePaired over Socket.IO to the UI.
 */
import createDebug from 'debug';
import { QRCodeSVG } from 'qrcode.react';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../../services/coreRpcClient';
import Button from '../../../ui/Button';

const log = createDebug('app:devices-ui:pair-modal');

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface CreatePairingResponse {
  channel_id: string;
  pairing_token: string;
  core_pubkey: string;
  rpc_url: string | null;
  expires_at: string;
}

interface PairedDevice {
  channel_id: string;
  label: string;
  peer_online: boolean | null;
  revoked: boolean;
}

interface ListDevicesResponse {
  devices: PairedDevice[];
}

type ModalState =
  | { kind: 'loading' }
  | { kind: 'qr'; session: CreatePairingResponse; qrUrl: string; expired: boolean }
  | { kind: 'success'; channelId: string; label: string }
  | { kind: 'error'; message: string };

interface PairPhoneModalProps {
  onClose: () => void;
  /** Called when a device successfully completes pairing. */
  onPaired: (channelId: string) => void;
}

// ---------------------------------------------------------------------------
// QR URL builder
// ---------------------------------------------------------------------------

function buildPairUrl(session: CreatePairingResponse): string {
  const params = new URLSearchParams();
  params.set('cid', session.channel_id);
  params.set('pt', session.pairing_token);
  params.set('cpk', session.core_pubkey);
  if (session.rpc_url) params.set('rpc', session.rpc_url);
  // expires_at is ISO 8601 — convert to unix timestamp for compact QR.
  const expUnix = Math.floor(new Date(session.expires_at).getTime() / 1_000);
  params.set('exp', String(expUnix));
  return `openhuman://pair?${params.toString()}`;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const PairPhoneModal = ({ onClose, onPaired }: PairPhoneModalProps) => {
  const { t } = useT();
  const [state, setState] = useState<ModalState>({ kind: 'loading' });
  const [showDetails, setShowDetails] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const expireTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pairedRef = useRef(false);

  const clearTimers = () => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
    if (expireTimerRef.current) {
      clearTimeout(expireTimerRef.current);
      expireTimerRef.current = null;
    }
  };

  // Watch the paired-device list to detect handshake completion.
  const startPollForPaired = useCallback(
    (channelId: string) => {
      if (pollRef.current) return;
      log('[devices-ui] [pair-modal] starting poll for channel_id=%s', channelId);
      pollRef.current = setInterval(async () => {
        if (pairedRef.current) return;
        try {
          const res = await callCoreRpc<ListDevicesResponse>({
            method: 'openhuman.devices_list',
            params: {},
          });
          const matched = res.devices.find(d => d.channel_id === channelId && !d.revoked);
          if (matched) {
            pairedRef.current = true;
            clearTimers();
            log(
              '[devices-ui] [pair-modal] device paired! channel_id=%s label=%s',
              channelId,
              matched.label
            );
            setState({ kind: 'success', channelId, label: matched.label });
            // Auto-close after 3 s to let the user read the success message.
            setTimeout(() => {
              onPaired(channelId);
            }, 3_000);
          }
        } catch (err) {
          // Non-fatal poll failure — the modal stays open.
          log('[devices-ui] [pair-modal] poll error: %s', String(err));
        }
      }, 2_000);
    },
    [onPaired]
  );

  const createSession = useCallback(async () => {
    clearTimers();
    pairedRef.current = false;
    setState({ kind: 'loading' });
    log('[devices-ui] [pair-modal] calling devices_create_pairing');
    try {
      const session = await callCoreRpc<CreatePairingResponse>({
        method: 'openhuman.devices_create_pairing',
        params: {},
      });
      log(
        '[devices-ui] [pair-modal] session created channel_id=%s token_len=%d expires_at=%s',
        session.channel_id,
        session.pairing_token.length,
        session.expires_at
      );
      const qrUrl = buildPairUrl(session);
      setState({ kind: 'qr', session, qrUrl, expired: false });

      // Schedule expiry transition.
      const msUntilExpiry = new Date(session.expires_at).getTime() - Date.now();
      if (msUntilExpiry > 0) {
        expireTimerRef.current = setTimeout(() => {
          log('[devices-ui] [pair-modal] QR expired channel_id=%s', session.channel_id);
          setState(prev =>
            prev.kind === 'qr' && prev.session.channel_id === session.channel_id
              ? { ...prev, expired: true }
              : prev
          );
        }, msUntilExpiry);
      }

      startPollForPaired(session.channel_id);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('[devices-ui] [pair-modal] create pairing error: %s', msg);
      setState({
        kind: 'error',
        message: t('devices.pairModal.errorPrefix').replace('{message}', msg),
      });
    }
  }, [startPollForPaired, t]);

  useEffect(() => {
    void createSession();
    return clearTimers;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/30">
      <div className="bg-surface rounded-2xl max-w-sm w-full border border-line shadow-large overflow-hidden">
        {/* Header — keep modal structure intact */}
        <div className="flex items-center justify-between px-5 pt-5 pb-3 border-b border-line-subtle">
          <h3 className="text-base font-semibold text-content">{t('devices.pairModal.title')}</h3>
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            onClick={onClose}
            aria-label={t('common.close')}
            className="w-6 h-6 rounded-full p-0">
            <svg
              className="w-4 h-4 text-content-muted"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </Button>
        </div>

        {/* Body */}
        <div className="p-5">
          {state.kind === 'loading' && <LoadingBody />}
          {state.kind === 'error' && (
            <ErrorBody
              message={state.message}
              onRetry={() => {
                void createSession();
              }}
            />
          )}
          {state.kind === 'qr' && !state.expired && (
            <QrBody
              session={state.session}
              qrUrl={state.qrUrl}
              showDetails={showDetails}
              onToggleDetails={() => setShowDetails(v => !v)}
            />
          )}
          {state.kind === 'qr' && state.expired && (
            <ExpiredBody
              onRegenerate={() => {
                void createSession();
              }}
            />
          )}
          {state.kind === 'success' && (
            <SuccessBody label={state.label} channelId={state.channelId} />
          )}
        </div>

        {/* Footer */}
        {(state.kind === 'qr' || state.kind === 'error') && (
          <div className="px-5 pb-5">
            <Button type="button" variant="tertiary" size="md" onClick={onClose} className="w-full">
              {t('common.cancel')}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// State-specific sub-components
// ---------------------------------------------------------------------------

function LoadingBody() {
  const { t } = useT();
  return (
    <div className="flex flex-col items-center justify-center py-10 gap-3">
      <svg className="w-6 h-6 animate-spin text-primary-400" fill="none" viewBox="0 0 24 24">
        <circle
          className="opacity-25"
          cx="12"
          cy="12"
          r="10"
          stroke="currentColor"
          strokeWidth="4"
        />
        <path
          className="opacity-75"
          fill="currentColor"
          d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
        />
      </svg>
      <p className="text-sm text-content-muted">{t('devices.pairModal.loading')}</p>
    </div>
  );
}

function QrBody({
  session,
  qrUrl,
  showDetails,
  onToggleDetails,
}: {
  session: CreatePairingResponse;
  qrUrl: string;
  showDetails: boolean;
  onToggleDetails: () => void;
}) {
  const { t } = useT();
  const expiresAt = new Date(session.expires_at);
  const minutesLeft = Math.max(0, Math.floor((expiresAt.getTime() - Date.now()) / 60_000));

  return (
    <div className="flex flex-col items-center gap-4">
      <p className="text-sm text-content-secondary text-center">
        {t('devices.pairModal.instructions')}
      </p>

      {/* QR code */}
      <div className="p-3 bg-surface rounded-xl border border-line shadow-sm">
        <QRCodeSVG value={qrUrl} size={200} level="M" bgColor="#ffffff" fgColor="#1c1917" />
      </div>

      <p className="text-xs text-content-faint">
        {t(
          minutesLeft === 1 ? 'devices.pairModal.expiresIn' : 'devices.pairModal.expiresInPlural'
        ).replace('{count}', String(minutesLeft))}
      </p>

      {/* Details toggle */}
      <Button
        type="button"
        variant="tertiary"
        size="xs"
        onClick={onToggleDetails}
        className="text-primary-500 hover:text-primary-600">
        {showDetails ? t('devices.pairModal.hideDetails') : t('devices.pairModal.showDetails')}
      </Button>

      {showDetails && (
        <div className="w-full space-y-2">
          <div>
            <p className="text-xs font-medium text-content-muted mb-1">
              {t('devices.pairModal.channelId')}
            </p>
            <p className="text-xs font-mono text-content-secondary bg-surface-muted rounded px-2 py-1 break-all select-all">
              {session.channel_id}
            </p>
          </div>
          <div>
            <p className="text-xs font-medium text-content-muted mb-1">
              {t('devices.pairModal.pairingUrl')}
            </p>
            <div className="relative">
              <p className="text-xs font-mono text-content-secondary bg-surface-muted rounded px-2 py-1 break-all select-all pr-16">
                {qrUrl}
              </p>
              <Button
                type="button"
                variant="tertiary"
                size="xs"
                onClick={() => {
                  void navigator.clipboard.writeText(qrUrl);
                }}
                className="absolute top-1 right-1 bg-surface border border-line dark:border-line-strong">
                {t('devices.pairModal.copyUrl')}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function ExpiredBody({ onRegenerate }: { onRegenerate: () => void }) {
  const { t } = useT();
  return (
    <div className="flex flex-col items-center gap-4 py-4">
      <div className="w-12 h-12 rounded-xl bg-amber-50 flex items-center justify-center">
        <svg
          className="w-6 h-6 text-amber-500"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.5}
            d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
      </div>
      <p className="text-sm font-medium text-content-secondary">
        {t('devices.pairModal.expiredTitle')}
      </p>
      <p className="text-xs text-content-muted text-center">{t('devices.pairModal.expiredBody')}</p>
      <Button type="button" variant="primary" size="md" onClick={onRegenerate}>
        {t('devices.pairModal.generateNewCode')}
      </Button>
    </div>
  );
}

function SuccessBody({ label, channelId }: { label: string; channelId: string }) {
  const { t } = useT();
  return (
    <div className="flex flex-col items-center gap-4 py-4">
      <div className="w-12 h-12 rounded-xl bg-sage-50 flex items-center justify-center">
        <svg
          className="w-6 h-6 text-sage-500"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
        </svg>
      </div>
      <div className="text-center">
        <p className="text-sm font-medium text-content">{t('devices.pairModal.successTitle')}</p>
        <p className="text-xs text-content-muted mt-1">{label}</p>
        <p className="text-xs font-mono text-content-faint mt-0.5">
          {channelId.slice(0, 8)}…{channelId.slice(-6)}
        </p>
      </div>
      <p className="text-xs text-content-faint">{t('devices.pairModal.autoClose')}</p>
    </div>
  );
}

function ErrorBody({ message, onRetry }: { message: string; onRetry: () => void }) {
  const { t } = useT();
  return (
    <div className="flex flex-col items-center gap-4 py-4">
      <div className="w-12 h-12 rounded-xl bg-coral-50 flex items-center justify-center">
        <svg
          className="w-6 h-6 text-coral-500"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M6 18L18 6M6 6l12 12"
          />
        </svg>
      </div>
      <p className="text-sm font-medium text-content-secondary">
        {t('devices.pairModal.errorTitle')}
      </p>
      <p className="text-xs text-content-muted text-center break-all">{message}</p>
      <Button type="button" variant="primary" size="md" onClick={onRetry}>
        {t('common.retry')}
      </Button>
    </div>
  );
}

export default PairPhoneModal;
