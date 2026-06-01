/**
 * Detail view for a single installed MCP server.
 * Shows header, status, env key names (never values), tool list, and action buttons.
 */
import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import ConfigAssistantPanel from './ConfigAssistantPanel';
import McpStatusBadge from './McpStatusBadge';
import McpToolList from './McpToolList';
import McpToolPlayground from './McpToolPlayground';
import type { ConnStatus, InstalledServer, McpTool, ServerStatus } from './types';

const log = debug('mcp-clients:detail');

interface InstalledServerDetailProps {
  server: InstalledServer;
  connStatus: ConnStatus | undefined;
  onUninstalled: (serverId: string) => void;
}

const InstalledServerDetail = ({
  server,
  connStatus,
  onUninstalled,
}: InstalledServerDetailProps) => {
  const { t } = useT();
  const status: ServerStatus = connStatus?.status ?? 'disconnected';
  const [tools, setTools] = useState<McpTool[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const [showAssistant, setShowAssistant] = useState(false);
  // Reconfigure form: when open, renders one input per env key so the user can
  // supply replacement values and reconnect without uninstall/reinstall
  // (issue #3039 env-reconfiguration). Values are never pre-filled from the
  // server (we only ever hold key names) — except when the config assistant
  // suggests values, which seed `reconfigValues` for the user to confirm.
  const [reconfigOpen, setReconfigOpen] = useState(false);
  const [reconfigValues, setReconfigValues] = useState<Record<string, string>>({});
  const [showReconfig, setShowReconfig] = useState<Record<string, boolean>>({});
  const [reconfigDone, setReconfigDone] = useState(false);
  // When non-null, the Tool Execution Playground modal is rendered for
  // this tool. Cleared on close. Only meaningful while the server is
  // connected (the gate is enforced at the McpToolList rendering site).
  const [playgroundTool, setPlaygroundTool] = useState<McpTool | null>(null);

  // Poll-driven safety net: if the server leaves `connected` by ANY path —
  // background status poll, parent prop change, auth expiry — not just the
  // explicit disconnect/uninstall handlers, drop the staged playground so its
  // now-unreachable tool can't be run AND doesn't spring back open when the
  // server reconnects. Implemented via React's "adjust state while rendering"
  // pattern (store the previous status, reset on change) rather than an
  // effect — same result without the extra render pass or the
  // set-state-in-effect lint. The render gate below is the belt-and-suspenders
  // guard for the single render before this runs.
  const [prevStatus, setPrevStatus] = useState(status);
  if (status !== prevStatus) {
    setPrevStatus(status);
    if (status !== 'connected' && playgroundTool) {
      setPlaygroundTool(null);
    }
  }

  const runBusy = useCallback(async (task: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await task();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('error: %s', msg);
      setError(msg);
    } finally {
      setBusy(false);
    }
  }, []);

  const handleConnect = useCallback(() => {
    void runBusy(async () => {
      log('connecting server_id=%s', server.server_id);
      const result = await mcpClientsApi.connect(server.server_id);
      setTools(result.tools);
      log('connected, %d tools', result.tools.length);
    });
  }, [server.server_id, runBusy]);

  const handleDisconnect = useCallback(() => {
    void runBusy(async () => {
      log('disconnecting server_id=%s', server.server_id);
      await mcpClientsApi.disconnect(server.server_id);
      // Clear stale tool list so it doesn't show after disconnection.
      setTools([]);
      // Drop any open Tool Execution Playground — its tool is no longer
      // reachable on this server. The render gate below ALSO enforces
      // this, but clearing the state here releases any in-flight async
      // work the modal was holding (history, copy timer, etc.).
      setPlaygroundTool(null);
      log('disconnected');
    });
  }, [server.server_id, runBusy]);

  const handleUninstall = useCallback(() => {
    void runBusy(async () => {
      log('uninstalling server_id=%s', server.server_id);
      await mcpClientsApi.uninstall(server.server_id);
      // The detail view is about to unmount via onUninstalled, but
      // clear explicitly so there's no window during which the modal
      // points at a now-removed server.
      setPlaygroundTool(null);
      log('uninstalled');
      onUninstalled(server.server_id);
    });
  }, [server.server_id, runBusy, onUninstalled]);

  const openReconfigure = useCallback(
    (prefill?: Record<string, string>) => {
      const initial: Record<string, string> = {};
      const initialVisibility: Record<string, boolean> = {};
      for (const key of server.env_keys) {
        initial[key] = prefill?.[key] ?? '';
        initialVisibility[key] = false;
      }
      setReconfigValues(initial);
      setShowReconfig(initialVisibility);
      setReconfigDone(false);
      setReconfigOpen(true);
    },
    [server.env_keys]
  );

  // The config assistant suggests values — seed the reconfigure form with them
  // so the user can confirm/complete before we persist + reconnect. Suggested
  // sets may be partial; the form requires every key so a reconnect never drops
  // a required var (issue #3039 gap B6 — suggested values were never persisted).
  const handleApplySuggestedEnv = useCallback(
    (env: Record<string, string>) => {
      log('suggested_env received, opening reconfigure form keys=%o', Object.keys(env));
      setShowAssistant(false);
      openReconfigure(env);
    },
    [openReconfigure]
  );

  const handleSaveReconfigure = useCallback(() => {
    void runBusy(async () => {
      // Replace-all semantics (update_env DELETEs then INSERTs): every key must
      // have a value or the server loses required env on reconnect. Mirror the
      // install dialog's validation.
      for (const key of server.env_keys) {
        if (!reconfigValues[key]?.trim()) {
          throw new Error(t('mcp.install.missingRequired').replace('{key}', key));
        }
      }
      log('reconfigure save server_id=%s', server.server_id);
      const result = await mcpClientsApi.updateEnv({
        server_id: server.server_id,
        env: reconfigValues,
      });
      setTools(result.tools ?? []);
      if (result.status !== 'connected') {
        throw new Error(result.error ?? t('mcp.detail.reconfigureReconnectFailed'));
      }
      setReconfigDone(true);
      setReconfigOpen(false);
    });
  }, [server.env_keys, server.server_id, reconfigValues, runBusy, t]);

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-start gap-3">
        {server.icon_url ? (
          <img
            src={server.icon_url}
            alt=""
            className="w-10 h-10 rounded shrink-0 object-contain bg-white dark:bg-neutral-900 border border-stone-100 dark:border-neutral-800"
          />
        ) : (
          <div className="w-10 h-10 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-lg">
            🔌
          </div>
        )}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {server.display_name}
            </h3>
            <McpStatusBadge status={status} />
          </div>
          {server.description && (
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5">
              {server.description}
            </p>
          )}
          <p className="text-[11px] text-stone-400 dark:text-neutral-500 mt-1 font-mono">
            {server.qualified_name}
          </p>
        </div>
      </div>

      {/* Error */}
      {(error || connStatus?.last_error) && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {error ?? connStatus?.last_error}
        </div>
      )}

      {/* Reconfigure success notice */}
      {reconfigDone && (
        <div className="rounded-lg border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-4 py-3 text-sm text-sage-700 dark:text-sage-300">
          {t('mcp.detail.reconfigureSuccess')}
        </div>
      )}

      {/* Action buttons */}
      <div className="flex flex-wrap gap-2">
        {status !== 'connected' ? (
          <button
            type="button"
            disabled={busy || status === 'connecting'}
            onClick={handleConnect}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
            {status === 'connecting' ? t('mcp.detail.connecting') : t('mcp.detail.connect')}
          </button>
        ) : (
          <button
            type="button"
            disabled={busy}
            onClick={handleDisconnect}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
            {t('mcp.detail.disconnect')}
          </button>
        )}

        <button
          type="button"
          disabled={busy}
          onClick={() => setShowAssistant(prev => !prev)}
          className="rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
          {showAssistant ? t('mcp.detail.hideAssistant') : t('mcp.detail.helpConfigure')}
        </button>

        {confirmUninstall ? (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-coral-600 dark:text-coral-400 font-medium">
              {t('mcp.detail.confirmUninstall')}
            </span>
            <button
              type="button"
              disabled={busy}
              onClick={handleUninstall}
              className="rounded-lg bg-coral-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-coral-600 disabled:opacity-50">
              {t('mcp.detail.confirmUninstallAction')}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirmUninstall(false)}
              className="rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 disabled:opacity-50">
              {t('common.cancel')}
            </button>
          </div>
        ) : (
          <button
            type="button"
            disabled={busy}
            onClick={() => setConfirmUninstall(true)}
            className="rounded-lg border border-coral-200 dark:border-coral-500/30 px-3 py-1.5 text-xs font-medium text-coral-600 dark:text-coral-400 hover:bg-coral-50 dark:hover:bg-coral-500/10 disabled:opacity-50">
            {t('mcp.detail.uninstall')}
          </button>
        )}
      </div>

      {/* Env keys (names only) + reconfigure affordance */}
      {server.env_keys.length > 0 && (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <p className="text-xs font-medium text-stone-600 dark:text-neutral-400">
              {t('mcp.detail.envVars')}
            </p>
            <button
              type="button"
              disabled={busy}
              onClick={() => (reconfigOpen ? setReconfigOpen(false) : openReconfigure())}
              className="text-[11px] font-medium text-primary-600 dark:text-primary-400 hover:underline disabled:opacity-50">
              {reconfigOpen ? t('common.cancel') : t('mcp.detail.reconfigure')}
            </button>
          </div>
          {!reconfigOpen && (
            <div className="flex flex-wrap gap-1.5">
              {server.env_keys.map(key => (
                <span
                  key={key}
                  className="px-2 py-0.5 text-[11px] font-mono rounded bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 border border-stone-200 dark:border-neutral-700">
                  {key}
                </span>
              ))}
            </div>
          )}
          {reconfigOpen && (
            <div className="space-y-2 rounded-lg border border-stone-200 dark:border-neutral-800 p-3">
              <p className="text-[11px] text-stone-500 dark:text-neutral-400">
                {t('mcp.detail.reconfigureHint')}
              </p>
              {server.env_keys.map(key => (
                <div key={key} className="space-y-1">
                  <label
                    htmlFor={`reconfig-${key}`}
                    className="block text-[11px] font-medium text-stone-600 dark:text-neutral-400">
                    {key}
                  </label>
                  <div className="flex gap-2">
                    <input
                      id={`reconfig-${key}`}
                      type={showReconfig[key] ? 'text' : 'password'}
                      value={reconfigValues[key] ?? ''}
                      onChange={e =>
                        setReconfigValues(prev => ({ ...prev, [key]: e.target.value }))
                      }
                      placeholder={t('mcp.install.enterValue').replace('{key}', key)}
                      disabled={busy}
                      className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
                    />
                    <button
                      type="button"
                      onClick={() => setShowReconfig(prev => ({ ...prev, [key]: !prev[key] }))}
                      disabled={busy}
                      className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-2 py-1 text-[11px] text-stone-500 dark:text-neutral-400 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
                      {showReconfig[key] ? t('mcp.install.hide') : t('mcp.install.show')}
                    </button>
                  </div>
                </div>
              ))}
              <button
                type="button"
                disabled={busy}
                onClick={handleSaveReconfigure}
                className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
                {busy ? t('mcp.detail.reconfigureSaving') : t('mcp.detail.saveReconnect')}
              </button>
            </div>
          )}
        </div>
      )}

      {/* Tool list — only show when connected so stale tools don't linger.
          When connected, each tool gets a "Try" button via `onTryTool`
          that opens the Tool Execution Playground modal below. */}
      <div className="space-y-1">
        <p className="text-xs font-medium text-stone-600 dark:text-neutral-400">
          {t('mcp.detail.tools')}
        </p>
        <McpToolList
          tools={status === 'connected' ? tools : []}
          onTryTool={status === 'connected' ? setPlaygroundTool : undefined}
        />
      </div>

      {/* Config assistant */}
      {showAssistant && (
        <div className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3">
          <ConfigAssistantPanel
            qualifiedName={server.qualified_name}
            onApplySuggestedEnv={handleApplySuggestedEnv}
          />
        </div>
      )}

      {/* Tool Execution Playground modal. Gated on BOTH a selected tool
          AND a live connection — a disconnected server's tool list is
          stale by definition, and the upstream RPC will reject calls
          anyway. The handlers above also clear `playgroundTool` on
          explicit disconnect / uninstall; this gate is the safety net
          for any state path that flips `status` without going through
          those handlers (poll-driven status change, parent forcing a
          reconnect, etc.). */}
      {playgroundTool && status === 'connected' && (
        <McpToolPlayground
          serverId={server.server_id}
          tool={playgroundTool}
          onClose={() => setPlaygroundTool(null)}
        />
      )}
    </div>
  );
};

export default InstalledServerDetail;
