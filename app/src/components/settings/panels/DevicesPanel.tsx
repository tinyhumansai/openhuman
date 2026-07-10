import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import type { ToastNotification } from '../../../types/intelligence';
import { ToastContainer } from '../../intelligence/Toast';
import Button from '../../ui/Button';
import {
  SettingsBadge,
  SettingsEmptyState,
  SettingsListItem,
  SettingsSection,
  SettingsStatusLine,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
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
      data-testid={isOnline ? 'peer-status-online' : 'peer-status-offline'}
      className={`inline-block w-2 h-2 rounded-full flex-shrink-0 ${isOnline ? 'bg-sage-500' : 'bg-neutral-300'}`}
    />
  );
}

function DeviceRow({
  device,
  onRevoke,
}: {
  device: PairedDevice;
  onRevoke: (device: PairedDevice) => void;
}) {
  const { t } = useT();
  const statusBadge = (
    <div className="flex items-center gap-1.5">
      <PeerDot online={device.peer_online} />
      <span className="font-mono text-xs text-content-faint">{truncateId(device.channel_id)}</span>
      <span className="text-xs text-content-faint">
        {formatRelativeTime(relativeTime(device.last_seen_at), t)}
      </span>
    </div>
  );

  return (
    <SettingsListItem
      label={device.label}
      badge={statusBadge}
      onRemove={() => onRevoke(device)}
      removeLabel={t('devices.revoke')}
    />
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
      <div className="bg-surface rounded-2xl max-w-sm w-full p-6 border border-line shadow-large">
        <h3 className="text-base font-semibold text-content mb-2">
          {t('devices.confirmRevokeTitle')}
        </h3>
        <p className="text-sm text-content-secondary mb-5">
          {t('devices.confirmRevokeBody').replace('{label}', device.label)}
        </p>
        <div className="flex gap-3">
          <Button type="button" variant="secondary" size="md" onClick={onCancel} className="flex-1">
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            tone="danger"
            size="md"
            onClick={onConfirm}
            className="flex-1"
            data-testid="confirm-revoke-btn">
            {t('devices.revoke')}
          </Button>
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
    <SettingsPanel
      description={t('settings.account.devicesDesc')}
      action={
        <Button type="button" variant="primary" size="xs" onClick={handleOpenPairModal}>
          {t('devices.pairIphone')}
        </Button>
      }>
      <div className="pb-3 flex items-center gap-2">
        {/* Bespoke beta badge — intentional marketing chip */}
        <SettingsBadge variant="warning">{t('devices.betaBadge')}</SettingsBadge>
        <p className="text-xs text-content-muted">{t('devices.betaText')}</p>
      </div>

      <div className="pb-5 space-y-3">
        {loading && (
          <div className="flex items-center justify-center py-12">
            <svg
              className="w-5 h-5 animate-spin text-content-faint"
              fill="none"
              viewBox="0 0 24 24">
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

        {!loading && error && <SettingsStatusLine saving={false} error={error} savingLabel="" />}

        {!loading && !error && devices.length === 0 && (
          <SettingsSection>
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <div className="w-12 h-12 rounded-xl bg-primary-50 dark:bg-primary-500/10 flex items-center justify-center mb-3">
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
              <SettingsEmptyState label={t('devices.noPaired')} />
              <p className="text-xs text-content-faint mb-4 max-w-xs">{t('devices.emptyState')}</p>
              <Button type="button" variant="primary" size="sm" onClick={handleOpenPairModal}>
                {t('devices.pairIphone')}
              </Button>
            </div>
          </SettingsSection>
        )}

        {!loading && !error && devices.length > 0 && (
          <SettingsSection>
            <ul>
              {devices.map(device => (
                <DeviceRow
                  key={device.channel_id}
                  device={device}
                  onRevoke={d => {
                    log('[devices-ui] revoke requested channel_id=%s', d.channel_id);
                    setRevokeTarget(d);
                  }}
                />
              ))}
            </ul>
          </SettingsSection>
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
    </SettingsPanel>
  );
};

export default DevicesPanel;
