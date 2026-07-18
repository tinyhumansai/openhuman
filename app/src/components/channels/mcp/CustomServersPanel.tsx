/**
 * CustomServersPanel — manage hand-added MCP servers.
 *
 * Sits alongside the registry catalog on the MCP Servers tab. The catalog table
 * covers servers a registry publishes; this pane covers the ones it doesn't —
 * a local command or a private endpoint the user enters themselves.
 *
 * It renders from the same `servers` / `statuses` arrays the tab already polls
 * rather than fetching its own, so the two views can never disagree about what
 * is installed or connected. Mutations delegate refresh to `onChanged`, which
 * is the tab's existing reload.
 */
import createDebug from 'debug';
import { useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { type CustomServerParams, mcpClientsApi } from '../../../services/api/mcpClientsApi';
import Button from '../../ui/Button';
import CustomServerFormModal from './CustomServerFormModal';
import McpStatusBadge from './McpStatusBadge';
import { type ConnStatus, type InstalledServer, isCustomServer, type ServerStatus } from './types';

const log = createDebug('app:mcp:CustomServersPanel');

export interface CustomServersPanelProps {
  /** Every installed server; the panel filters to the custom ones itself. */
  servers: InstalledServer[];
  statuses: ConnStatus[];
  /** Reload installed servers + statuses after a mutation. */
  onChanged: () => Promise<void>;
  /** Open a server's detail view (tools, logs, playground). */
  onSelectServer: (serverId: string) => void;
}

type FormState = { mode: 'create' } | { mode: 'edit'; server: InstalledServer } | null;

const transportLabelKey = (server: InstalledServer): string =>
  server.transport?.kind === 'http_remote'
    ? 'mcp.custom.transport.remote'
    : 'mcp.custom.transport.local';

const CustomServersPanel = ({
  servers,
  statuses,
  onChanged,
  onSelectServer,
}: CustomServersPanelProps) => {
  const { t } = useT();
  const [form, setForm] = useState<FormState>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const customServers = useMemo(() => servers.filter(isCustomServer), [servers]);

  const statusFor = (serverId: string): ServerStatus =>
    statuses.find(s => s.server_id === serverId)?.status ?? 'disconnected';

  // The refresh (`onChanged`) is wrapped everywhere below: persistence has
  // already committed by the time we refresh, so a reload failure must not
  // reject back into the form and report the add/edit as failed (letting the
  // user "retry" an operation that already succeeded). Refresh failures are
  // logged, not surfaced as mutation failures.
  const safeRefresh = async (stage: string, serverId: string) => {
    try {
      await onChanged();
    } catch (err) {
      log('%s refresh failed for %s: %o', stage, serverId, err);
    }
  };

  const handleAdd = async (params: CustomServerParams) => {
    log('add: begin transport=%s', params.transport);
    const server = await mcpClientsApi.addCustom(params);
    log('add: persisted server_id=%s', server.server_id);
    // Show the row immediately, then dial.
    await safeRefresh('post-add', server.server_id);
    // Dial through the ordinary connect path so a bad command or URL surfaces
    // the same status/error the rest of the tab already renders. A failure here
    // is not an add failure — the row exists and is editable — so it must not
    // reject back into the form.
    try {
      await mcpClientsApi.connect(server.server_id);
      log('add: connected server_id=%s', server.server_id);
    } catch (err) {
      log('add: post-add connect failed for %s: %o', server.server_id, err);
    }
    await safeRefresh('post-add-connect', server.server_id);
    log('add: done server_id=%s', server.server_id);
  };

  const handleEdit = async (server: InstalledServer, params: CustomServerParams) => {
    log('edit: begin server_id=%s transport=%s', server.server_id, params.transport);
    await mcpClientsApi.updateCustom({ ...params, server_id: server.server_id });
    log('edit: persisted server_id=%s', server.server_id);
    await safeRefresh('post-edit', server.server_id);
    // update_custom drops the live connection so the next dial uses the new
    // settings; re-dial here so the row doesn't sit disconnected after an edit.
    try {
      await mcpClientsApi.connect(server.server_id);
      log('edit: reconnected server_id=%s', server.server_id);
    } catch (err) {
      log('edit: post-edit connect failed for %s: %o', server.server_id, err);
    }
    await safeRefresh('post-edit-connect', server.server_id);
    log('edit: done server_id=%s', server.server_id);
  };

  const handleRemove = async (server: InstalledServer) => {
    setError(null);
    setBusyId(server.server_id);
    try {
      await mcpClientsApi.uninstall(server.server_id);
      log('removed server_id=%s', server.server_id);
      await onChanged();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('remove failed for %s: %s', server.server_id, msg);
      setError(msg);
    } finally {
      setBusyId(null);
    }
  };

  return (
    <section
      className="rounded-lg border border-line bg-surface"
      aria-labelledby="mcp-custom-servers-heading"
      data-testid="mcp-custom-servers-panel">
      <div className="flex items-center justify-between gap-2 border-b border-line-subtle px-3 py-2.5">
        <div className="min-w-0">
          <h3 id="mcp-custom-servers-heading" className="text-sm font-semibold text-content">
            {t('mcp.custom.title')}
          </h3>
          <p className="text-xs text-content-muted">{t('mcp.custom.subtitle')}</p>
        </div>
        <Button
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick={() => setForm({ mode: 'create' })}>
          {t('mcp.custom.addButton')}
        </Button>
      </div>

      {error ? (
        <p
          role="alert"
          className="mx-3 mt-3 rounded-lg border border-coral-300 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:bg-coral-500/10">
          {error}
        </p>
      ) : null}

      {customServers.length === 0 ? (
        <p className="px-3 py-6 text-center text-xs text-content-muted">{t('mcp.custom.empty')}</p>
      ) : (
        <ul className="divide-y divide-line-subtle">
          {customServers.map(server => (
            <li key={server.server_id} className="px-3 py-2.5">
              <div className="flex items-start justify-between gap-2">
                <button
                  type="button"
                  onClick={() => onSelectServer(server.server_id)}
                  className="min-w-0 flex-1 text-left focus:outline-none focus:ring-2 focus:ring-primary-500/40 rounded">
                  <span className="block truncate text-sm font-medium text-content">
                    {server.display_name}
                  </span>
                  <span className="mt-0.5 flex items-center gap-1.5">
                    <McpStatusBadge status={statusFor(server.server_id)} />
                    <span className="text-xs text-content-muted">
                      {t(transportLabelKey(server))}
                    </span>
                  </span>
                </button>
                <div className="flex shrink-0 items-center gap-1">
                  <Button
                    variant="tertiary"
                    size="sm"
                    onClick={() => setForm({ mode: 'edit', server })}
                    aria-label={t('mcp.custom.editAria').replace('{name}', server.display_name)}>
                    {t('mcp.custom.edit')}
                  </Button>
                  <Button
                    variant="tertiary"
                    size="sm"
                    disabled={busyId === server.server_id}
                    onClick={() => void handleRemove(server)}
                    aria-label={t('mcp.custom.removeAria').replace('{name}', server.display_name)}>
                    {t('mcp.custom.remove')}
                  </Button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}

      {/* Branch on the discriminant so each variant supplies exactly its props
          (edit → server, create → none). Remount per target (`key`) so the form
          never shows a previous server's state — same reason CronJobsPanel keys
          its form modal. */}
      {form?.mode === 'edit' ? (
        <CustomServerFormModal
          key={form.server.server_id}
          mode="edit"
          server={form.server}
          onClose={() => setForm(null)}
          onSubmit={params => handleEdit(form.server, params)}
        />
      ) : form?.mode === 'create' ? (
        <CustomServerFormModal
          key="create"
          mode="create"
          onClose={() => setForm(null)}
          onSubmit={params => handleAdd(params)}
        />
      ) : null}
    </section>
  );
};

export default CustomServersPanel;
