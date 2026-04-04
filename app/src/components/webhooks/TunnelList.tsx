import { useState } from 'react';

import { type Tunnel, tunnelsApi } from '../../services/api/tunnelsApi';
import type { TunnelRegistration } from '../../store/webhooksSlice';
import { BACKEND_URL } from '../../utils/config';

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
      setActionError(err instanceof Error ? err.message : 'Failed to create tunnel');
    } finally {
      setCreating(false);
    }
  };

  const getRegistration = (uuid: string) => registrations.find(r => r.tunnel_uuid === uuid);

  const webhookUrl = (uuid: string) =>
    tunnelsApi.ingressUrl(BACKEND_URL || 'https://api.tinyhumans.ai', uuid);

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-stone-900">Webhook Tunnels</h3>
        <div className="flex gap-2">
          <button
            onClick={onRefresh}
            disabled={loading}
            className="px-3 py-1.5 text-sm text-stone-600 hover:text-stone-900 rounded-lg hover:bg-stone-100 transition-colors">
            {loading ? 'Loading...' : 'Refresh'}
          </button>
          <button
            onClick={() => setShowCreate(true)}
            className="px-3 py-1.5 text-sm font-medium text-white bg-primary-500 rounded-lg hover:bg-primary-600 transition-colors">
            + New Tunnel
          </button>
        </div>
      </div>

      {/* Create form */}
      {showCreate && (
        <div className="p-4 rounded-xl border border-stone-200 bg-white space-y-3">
          <input
            type="text"
            placeholder="Tunnel name (e.g. telegram-bot)"
            value={newName}
            onChange={e => setNewName(e.target.value)}
            className="w-full px-3 py-2 text-sm border border-stone-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500/30 focus:border-primary-500"
            autoFocus
          />
          <input
            type="text"
            placeholder="Description (optional)"
            value={newDesc}
            onChange={e => setNewDesc(e.target.value)}
            className="w-full px-3 py-2 text-sm border border-stone-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500/30 focus:border-primary-500"
          />
          <div className="flex gap-2 justify-end">
            <button
              onClick={() => setShowCreate(false)}
              className="px-3 py-1.5 text-sm text-stone-600 hover:text-stone-900 rounded-lg">
              Cancel
            </button>
            <button
              onClick={handleCreate}
              disabled={!newName.trim() || creating}
              className="px-3 py-1.5 text-sm font-medium text-white bg-primary-500 rounded-lg hover:bg-primary-600 disabled:opacity-50 transition-colors">
              {creating ? 'Creating...' : 'Create'}
            </button>
          </div>
        </div>
      )}

      {/* Error display */}
      {actionError && (
        <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm flex items-center justify-between">
          <span>{actionError}</span>
          <button
            onClick={() => setActionError(null)}
            className="text-coral-500 hover:text-coral-700 text-xs ml-2">
            Dismiss
          </button>
        </div>
      )}

      {/* Tunnel list */}
      {tunnels.length === 0 && !loading && (
        <p className="text-sm text-stone-500 text-center py-8">
          No tunnels yet. Create one to receive webhook events.
        </p>
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
      onError(err instanceof Error ? err.message : 'Failed to delete tunnel');
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
      onError(err instanceof Error ? err.message : 'Failed to toggle echo');
    } finally {
      setToggling(false);
    }
  };

  return (
    <div className="p-4 rounded-xl border border-stone-200 bg-white hover:border-stone-300 transition-colors">
      <div className="flex items-start justify-between">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h4 className="text-sm font-medium text-stone-900 truncate">{tunnel.name}</h4>
            <span
              className={`inline-flex items-center px-1.5 py-0.5 text-xs rounded-full ${
                tunnel.isActive ? 'bg-sage-100 text-sage-700' : 'bg-stone-100 text-stone-500'
              }`}>
              {tunnel.isActive ? 'Active' : 'Inactive'}
            </span>
            {isEchoRegistered && (
              <span className="inline-flex items-center px-1.5 py-0.5 text-xs rounded-full bg-amber-50 text-amber-700">
                Echo
              </span>
            )}
            {isSkillRegistered && registration && (
              <span className="inline-flex items-center px-1.5 py-0.5 text-xs rounded-full bg-primary-50 text-primary-700">
                {registration.skill_id}
              </span>
            )}
          </div>
          {tunnel.description && (
            <p className="mt-1 text-xs text-stone-500">{tunnel.description}</p>
          )}
          <div className="mt-2 flex items-center gap-2">
            <code className="text-xs text-stone-500 bg-stone-50 px-2 py-1 rounded font-mono truncate max-w-[400px]">
              {webhookUrl}
            </code>
            <button
              onClick={handleCopy}
              className="text-xs text-primary-500 hover:text-primary-700 whitespace-nowrap">
              {copied ? 'Copied!' : 'Copy'}
            </button>
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
              {toggling ? '...' : isEchoRegistered ? 'Remove Echo' : 'Enable Echo'}
            </button>
          )}
          <button
            onClick={handleDelete}
            disabled={deleting}
            className="px-2 py-1 text-xs text-coral-600 hover:text-coral-700 hover:bg-coral-50 rounded-lg transition-colors">
            {deleting ? '...' : 'Delete'}
          </button>
        </div>
      </div>
    </div>
  );
}
