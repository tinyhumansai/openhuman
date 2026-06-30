import { useState } from 'react';

import type { TunnelRegistration } from '../../features/webhooks/types';
import { useBackendUrl } from '../../hooks/useBackendUrl';
import { useT } from '../../lib/i18n/I18nContext';
import { type Tunnel, tunnelsApi } from '../../services/api/tunnelsApi';
import Button from '../ui/Button';

interface TunnelListProps {
  tunnels: Tunnel[];
  registrations: TunnelRegistration[];
  loading: boolean;
  onCreateTunnel: (name: string, description?: string) => Promise<Tunnel>;
  onDeleteTunnel: (id: string) => Promise<void>;
  onRefresh: () => Promise<void>;
  onRegisterEcho: (
    tunnelUuid: string,
    tunnelName?: string,
    backendTunnelId?: string
  ) => Promise<void>;
  onUnregisterEcho: (tunnelUuid: string) => Promise<void>;
}

export default function TunnelList({
  tunnels,
  registrations,
  loading,
  onCreateTunnel,
  onDeleteTunnel,
  onRefresh,
  onRegisterEcho,
  onUnregisterEcho,
}: TunnelListProps) {
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [creating, setCreating] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const backendUrl = useBackendUrl();
  const { t } = useT();

  const handleCreate = async () => {
    if (!newName.trim()) return;
    setCreating(true);
    setActionError(null);
    try {
      await onCreateTunnel(newName.trim(), newDesc.trim() || undefined);
      setNewName('');
      setNewDesc('');
      setShowCreate(false);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : t('webhooks.tunnels.createFailed'));
    } finally {
      setCreating(false);
    }
  };

  const getRegistration = (uuid: string) => registrations.find(r => r.tunnel_uuid === uuid);

  const webhookUrl = (uuid: string) => (backendUrl ? tunnelsApi.ingressUrl(backendUrl, uuid) : '');

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-content">{t('webhooks.tunnels.title')}</h3>
        <div className="flex gap-2">
          <Button variant="tertiary" size="sm" onClick={onRefresh} disabled={loading}>
            {loading ? t('common.loading') : t('common.refresh')}
          </Button>
          <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
            {t('webhooks.tunnels.newTunnel')}
          </Button>
        </div>
      </div>

      {/* Create form */}
      {showCreate && (
        <div className="p-4 rounded-xl border border-line bg-surface space-y-3">
          <input
            type="text"
            placeholder={t('webhooks.tunnels.namePlaceholder')}
            value={newName}
            onChange={e => setNewName(e.target.value)}
            className="w-full px-3 py-2 text-sm border border-line rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500/30 focus:border-primary-500"
            autoFocus
          />
          <input
            type="text"
            placeholder={t('webhooks.tunnels.descriptionPlaceholder')}
            value={newDesc}
            onChange={e => setNewDesc(e.target.value)}
            className="w-full px-3 py-2 text-sm border border-line rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500/30 focus:border-primary-500"
          />
          <div className="flex gap-2 justify-end">
            <Button variant="tertiary" size="sm" onClick={() => setShowCreate(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={handleCreate}
              disabled={!newName.trim() || creating}>
              {creating ? t('webhooks.tunnels.creating') : t('common.create')}
            </Button>
          </div>
        </div>
      )}

      {/* Error display */}
      {actionError && (
        <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm flex items-center justify-between">
          <span>{actionError}</span>
          <Button
            variant="tertiary"
            tone="danger"
            size="xs"
            onClick={() => setActionError(null)}
            className="ml-2">
            {t('common.dismiss')}
          </Button>
        </div>
      )}

      {/* Tunnel list */}
      {tunnels.length === 0 && !loading && (
        <p className="text-sm text-content-muted text-center py-8">{t('webhooks.tunnels.empty')}</p>
      )}

      <div className="space-y-2">
        {tunnels.map(tunnel => {
          const reg = getRegistration(tunnel.uuid);
          return (
            <TunnelCard
              key={tunnel.id}
              tunnel={tunnel}
              registration={reg}
              webhookUrl={webhookUrl(tunnel.uuid)}
              onDelete={() => onDeleteTunnel(tunnel.id)}
              onRegisterEcho={() => onRegisterEcho(tunnel.uuid, tunnel.name, tunnel.id)}
              onUnregisterEcho={() => onUnregisterEcho(tunnel.uuid)}
              onError={setActionError}
            />
          );
        })}
      </div>
    </div>
  );
}

// ── Tunnel Card ───────────────────────────────────────────────────────────────

interface TunnelCardProps {
  tunnel: Tunnel;
  registration?: TunnelRegistration;
  webhookUrl: string;
  onDelete: () => Promise<void>;
  onRegisterEcho: () => Promise<void>;
  onUnregisterEcho: () => Promise<void>;
  onError: (msg: string) => void;
}

function TunnelCard({
  tunnel,
  registration,
  webhookUrl,
  onDelete,
  onRegisterEcho,
  onUnregisterEcho,
  onError,
}: TunnelCardProps) {
  const { t } = useT();
  const [copied, setCopied] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [toggling, setToggling] = useState(false);

  const isEchoRegistered = registration?.target_kind === 'echo';
  const isSkillRegistered = registration?.target_kind === 'skill';

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(webhookUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard may not be available in Tauri WebView
    }
  };

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await onDelete();
    } catch (err) {
      onError(err instanceof Error ? err.message : t('webhooks.tunnels.deleteFailed'));
    } finally {
      setDeleting(false);
    }
  };

  const handleToggleEcho = async () => {
    setToggling(true);
    try {
      if (isEchoRegistered) {
        await onUnregisterEcho();
      } else {
        await onRegisterEcho();
      }
    } catch (err) {
      onError(err instanceof Error ? err.message : t('webhooks.tunnels.toggleFailed'));
    } finally {
      setToggling(false);
    }
  };

  return (
    <div className="p-4 rounded-xl border border-line bg-surface hover:border-line-strong transition-colors">
      <div className="flex items-start justify-between">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h4 className="text-sm font-medium text-content truncate">{tunnel.name}</h4>
            <span
              className={`inline-flex items-center px-1.5 py-0.5 text-xs rounded-full ${
                tunnel.isActive
                  ? 'bg-sage-100 text-sage-700'
                  : 'bg-surface-subtle text-content-muted'
              }`}>
              {tunnel.isActive ? t('webhooks.tunnels.active') : t('webhooks.tunnels.inactive')}
            </span>
            {isEchoRegistered && (
              <span className="inline-flex items-center px-1.5 py-0.5 text-xs rounded-full bg-amber-50 text-amber-700">
                {t('webhooks.tunnels.echo')}
              </span>
            )}
            {isSkillRegistered && registration && (
              <span className="inline-flex items-center px-1.5 py-0.5 text-xs rounded-full bg-primary-50 text-primary-700">
                {registration.skill_id}
              </span>
            )}
          </div>
          {tunnel.description && (
            <p className="mt-1 text-xs text-content-muted">{tunnel.description}</p>
          )}
          <div className="mt-2 flex items-center gap-2">
            <code className="text-xs text-content-muted bg-surface-muted px-2 py-1 rounded font-mono truncate max-w-[400px]">
              {webhookUrl || 'Resolving backend URL…'}
            </code>
            <Button
              variant="tertiary"
              size="xs"
              onClick={handleCopy}
              disabled={!webhookUrl}
              className="whitespace-nowrap">
              {copied ? t('common.copied') : t('common.copy')}
            </Button>
          </div>
        </div>
        <div className="ml-3 flex flex-col gap-1 shrink-0">
          {/* Echo toggle — only show if not already claimed by a skill */}
          {!isSkillRegistered && (
            <button
              onClick={handleToggleEcho}
              disabled={toggling}
              className={`px-2 py-1 text-xs rounded-lg transition-colors ${
                isEchoRegistered
                  ? 'text-amber-600 hover:text-amber-700 hover:bg-amber-50'
                  : 'text-primary-600 hover:text-primary-700 hover:bg-primary-50'
              }`}>
              {toggling
                ? '...'
                : isEchoRegistered
                  ? t('webhooks.tunnels.removeEcho')
                  : t('webhooks.tunnels.enableEcho')}
            </button>
          )}
          <Button
            variant="tertiary"
            tone="danger"
            size="xs"
            onClick={handleDelete}
            disabled={deleting}>
            {deleting ? '...' : t('common.delete')}
          </Button>
        </div>
      </div>
    </div>
  );
}
