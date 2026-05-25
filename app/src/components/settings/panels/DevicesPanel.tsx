import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import type { ToastNotification } from '../../../types/intelligence';
import { ToastContainer } from '../../intelligence/Toast';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import PairPhoneModal from './devices/PairPhoneModal';

const log = createDebug('app:devices-ui');

// ---------------------------------------------------------------------------
// Types (mirror the Rust types.rs)
// ---------------------------------------------------------------------------

export interface PairedDevice {
  channel_id: string;
  label: string;
  device_pubkey: string;
  created_at: string;
  last_seen_at: string | null;
  peer_online: boolean | null;
  revoked: boolean;
}

interface ListDevicesResponse {
  devices: PairedDevice[];
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function truncateId(id: string): string {
  if (id.length <= 10) return id;
  return `${id.slice(0, 4)}…${id.slice(-4)}`;
}

function relativeTime(iso: string | null): string {
  if (!iso) return 'Never';
  const delta = Date.now() - new Date(iso).getTime();
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 1) return 'Just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function formatRelativeTime(value: string, t: (key: string) => string): string {
  if (value === 'Never') return t('devices.lastSeenNever');
  if (value === 'Just now') return t('devices.lastSeenNow');

  const match = value.match(/^(\d+)([mhd]) ago$/);
  if (!match) return value;

  const [, count, unit] = match;
  const key =
    unit === 'm'
      ? 'devices.lastSeenMinutes'
      : unit === 'h'
        ? 'devices.lastSeenHours'
        : 'devices.lastSeenDays';

  return t(key).replace('{count}', count);
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function PeerDot({ online }: { online: boolean | null }) {
  const { t } = useT();
  const isOnline = online === true;
  return (
    <span
      title={isOnline ? t('devices.online') : t('devices.offline')}
      className={`inline-block w-2 h-2 rounded-full flex-shrink-0 ${isOnline ? 'bg-sage-500' : 'bg-stone-300'}`}
    />
  );
}

function DeviceRow({
  device,
  onRevoke,
  isFirst,
  isLast,
}: {
  device: PairedDevice;
  onRevoke: (device: PairedDevice) => void;
  isFirst: boolean;
  isLast: boolean;
}) {
  const { t } = useT();

  return (
    <div
      className={`flex items-center gap-3 px-4 py-3 bg-white border-b ${isLast ? 'border-b-0' : 'border-stone-100'} ${isFirst ? 'rounded-t-lg' : ''} ${isLast ? 'rounded-b-lg' : ''}`}>
      <PeerDot online={device.peer_online} />
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-stone-900 truncate">{device.label}</p>
        <p className="text-xs text-stone-400 font-mono">{truncateId(device.channel_id)}</p>
        <p className="text-xs text-stone-400">
          {formatRelativeTime(relativeTime(device.last_seen_at), t)}
        </p>
      </div>
      <button
        onClick={() => onRevoke(device)}
        className="text-xs text-coral-600 hover:text-coral-700 transition-colors flex-shrink-0 px-2 py-1 rounded hover:bg-coral-50"
        aria-label={t('devices.revokeAria').replace('{label}', device.label)}>
        {t('devices.revoke')}
      </button>
    </div>
  );
}

function ConfirmRevokeDialog({
  device,
  onConfirm,
  onCancel,
}: {
  device: PairedDevice;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useT();

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/30">
      <div className="bg-white rounded-2xl max-w-sm w-full p-6 border border-stone-200 shadow-large">
        <h3 className="text-base font-semibold text-stone-900 mb-2">
          {t('devices.confirmRevokeTitle')}
        </h3>
        <p className="text-sm text-stone-600 mb-5">
          {t('devices.confirmRevokeBody').replace('{label}', device.label)}
        </p>
        <div className="flex gap-3">
          <button
            onClick={onCancel}
            className="flex-1 px-4 py-2 rounded-lg border border-stone-200 text-stone-700 hover:bg-stone-50 transition-colors text-sm">
            {t('common.cancel')}
          </button>
          <button
            onClick={onConfirm}
            className="flex-1 px-4 py-2 rounded-lg bg-coral-500 hover:bg-coral-600 text-white transition-colors text-sm">
            {t('devices.revoke')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

const DevicesPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [devices, setDevices] = useState<PairedDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<PairedDevice | null>(null);
  const [revoking, setRevoking] = useState(false);
  const [showPairModal, setShowPairModal] = useState(false);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);

  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  }, []);

  // Import callCoreRpc lazily via module-level reference to avoid circular deps.
  const loadDevices = useCallback(async () => {
    log('[devices-ui] loadDevices start');
    setError(null);
    try {
      const res = await callCoreRpc<ListDevicesResponse>({
        method: 'openhuman.devices_list',
        params: {},
      });
      const active = res.devices.filter(d => !d.revoked);
      log('[devices-ui] loadDevices got %d device(s)', active.length);
      setDevices(active);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('[devices-ui] loadDevices error: %s', msg);
      setError(t('devices.loadFailed').replace('{message}', msg));
    } finally {
      setLoading(false);
    }
  }, [t]);

  // intervalRef keeps the poll alive when the pair modal is open.
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const startPolling = useCallback(() => {
    if (pollRef.current) return;
    pollRef.current = setInterval(() => {
      void loadDevices();
    }, 2_000);
    log('[devices-ui] started 2s poll for device updates');
  }, [loadDevices]);

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
      log('[devices-ui] stopped poll');
    }
  }, []);

  useEffect(() => {
    void loadDevices();
    return stopPolling;
  }, [loadDevices, stopPolling]);

  const handleOpenPairModal = () => {
    log('[devices-ui] opening pair modal');
    setShowPairModal(true);
    startPolling();
  };

  const handleClosePairModal = () => {
    log('[devices-ui] closing pair modal');
    setShowPairModal(false);
    stopPolling();
    void loadDevices();
  };

  const handlePaired = (channelId: string) => {
    log('[devices-ui] DevicePaired event channelId=%s', channelId);
    addToast({
      type: 'success',
      title: t('devices.devicePairedTitle'),
      message: t('devices.devicePairedMessage'),
    });
    stopPolling();
    setShowPairModal(false);
    void loadDevices();
  };

  const confirmRevoke = async () => {
    if (!revokeTarget) return;
    const target = revokeTarget;
    setRevoking(true);
    log('[devices-ui] revoking channel_id=%s', target.channel_id);
    try {
      await callCoreRpc({
        method: 'openhuman.devices_revoke',
        params: { channel_id: target.channel_id },
      });
      log('[devices-ui] revoke ok channel_id=%s', target.channel_id);
      addToast({
        type: 'success',
        title: t('devices.deviceRevokedTitle'),
        message: t('devices.deviceRevokedMessage').replace('{label}', target.label),
      });
      setRevokeTarget(null);
      await loadDevices();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('[devices-ui] revoke error: %s', msg);
      addToast({ type: 'error', title: t('devices.revokeFailedTitle'), message: msg });
    } finally {
      setRevoking(false);
    }
  };

  return (
    <div className="z-10 relative">
      <div className="flex items-center justify-between px-5 pt-5 pb-3">
        <SettingsHeader
          title={t('devices.title')}
          showBackButton={breadcrumbs.length > 0}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
        <button
          onClick={handleOpenPairModal}
          className="text-xs font-medium text-white bg-primary-500 hover:bg-primary-600 transition-colors px-3 py-1.5 rounded-lg flex-shrink-0">
          {t('devices.pairIphone')}
        </button>
      </div>

      <div className="px-5 pb-3 flex items-center gap-2">
        <span className="inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase tracking-wider bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-200 border border-amber-200 dark:border-amber-800/60">
          {t('devices.betaBadge')}
        </span>
        <p className="text-xs text-stone-500 dark:text-neutral-400">{t('devices.betaText')}</p>
      </div>

      <div className="px-5 pb-5">
        {loading && (
          <div className="flex items-center justify-center py-12">
            <svg className="w-5 h-5 animate-spin text-stone-400" fill="none" viewBox="0 0 24 24">
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
          </div>
        )}

        {!loading && error && (
          <div className="rounded-lg bg-coral-50 border border-coral-200 px-4 py-3 text-sm text-coral-700">
            {error}
          </div>
        )}

        {!loading && !error && devices.length === 0 && (
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <div className="w-12 h-12 rounded-xl bg-primary-50 flex items-center justify-center mb-3">
              <svg
                className="w-6 h-6 text-primary-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z"
                />
              </svg>
            </div>
            <p className="text-sm font-medium text-stone-700 mb-1">{t('devices.noPaired')}</p>
            <p className="text-xs text-stone-400 mb-4 max-w-xs">{t('devices.emptyState')}</p>
            <button
              onClick={handleOpenPairModal}
              className="px-4 py-2 text-sm font-medium text-white bg-primary-500 hover:bg-primary-600 transition-colors rounded-lg">
              {t('devices.pairIphone')}
            </button>
          </div>
        )}

        {!loading && !error && devices.length > 0 && (
          <div className="rounded-xl border border-stone-200 overflow-hidden">
            {devices.map((device, idx) => (
              <DeviceRow
                key={device.channel_id}
                device={device}
                onRevoke={d => {
                  log('[devices-ui] revoke requested channel_id=%s', d.channel_id);
                  setRevokeTarget(d);
                }}
                isFirst={idx === 0}
                isLast={idx === devices.length - 1}
              />
            ))}
          </div>
        )}
      </div>

      {revokeTarget && (
        <ConfirmRevokeDialog
          device={revokeTarget}
          onConfirm={() => {
            void confirmRevoke();
          }}
          onCancel={() => {
            if (!revoking) setRevokeTarget(null);
          }}
        />
      )}

      {showPairModal && <PairPhoneModal onClose={handleClosePairModal} onPaired={handlePaired} />}

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
};

export default DevicesPanel;
