/**
 * Detail view for a single installed MCP server.
 * Shows header, status, env key names (never values), tool list, and action buttons.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import Button from '../../ui/Button';
import { clearConfigChat } from './ConfigAssistantPanel';
import ConfigHelpModal from './ConfigHelpModal';
import ConnectAuthModal from './ConnectAuthModal';
import McpStatusBadge from './McpStatusBadge';
import McpToolList from './McpToolList';
import McpToolPlayground from './McpToolPlayground';
import type { ConnStatus, InstalledServer, McpTool, ServerStatus } from './types';

const log = debug('mcp-clients:detail');

// The "how do I get the token" AI assistant lives on this detail page (not the
// Connect modal, which stays focused on entering the credential). It auto-runs a
// server-specific prompt and can web-research the provider's docs.
const SHOW_CONFIG_ASSISTANT = true;

interface InstalledServerDetailProps {
  server: InstalledServer;
  connStatus: ConnStatus | undefined;
  onUninstalled: (serverId: string) => void;
  onEnabledChange?: (serverId: string, enabled: boolean) => void;
}

const InstalledServerDetail = ({
  server,
  connStatus,
  onUninstalled,
  onEnabledChange,
}: InstalledServerDetailProps) => {
  const { t } = useT();
  const status: ServerStatus = connStatus?.status ?? 'disconnected';
  // Internal bookkeeping keys (`__`-prefixed, e.g. the OAuth refresh bundle)
  // are hidden; for an OAuth-managed server the `Authorization` value is the
  // managed access token, so hide it too — the user re-authenticates via the
  // sign-in flow rather than editing it here.
  const oauthManaged = server.env_keys.includes('__oauth__');
  const visibleEnvKeys = server.env_keys.filter(
    k => !k.startsWith('__') && !(oauthManaged && k.toLowerCase() === 'authorization')
  );
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
  // When true, the upfront auth modal is shown so the user can supply tokens
  // (declared required fields + custom headers) before we actually connect.
  const [connectModalOpen, setConnectModalOpen] = useState(false);

  // The help chat persists (cached by qualified_name) while this server's detail
  // page is open, so closing/reopening the modal keeps the conversation. Clear
  // it when we leave this server (unmount → back to list, or switch servers).
  useEffect(() => () => clearConfigChat(server.qualified_name), [server.qualified_name]);

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

  // Connect now opens the upfront auth modal rather than dialing immediately,
  // so the user can supply credentials (declared fields + custom headers)
  // first. The modal performs the actual connect (via update_env or connect)
  // and hands back the tool list on success.
  const handleConnect = useCallback(() => {
    setError(null);
    setConnectModalOpen(true);
  }, []);

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

  const handleSetEnabled = useCallback(
    (next: boolean) => {
      void runBusy(async () => {
        log('set_enabled server_id=%s enabled=%s', server.server_id, next);
        await mcpClientsApi.setEnabled(server.server_id, next);
        if (!next) {
          // Disabling the server: drop stale tool list so no tool rows
          // remain in view while the server is disabled, and clear any
          // open playground session.
          setTools([]);
          setPlaygroundTool(null);
        }
        log('set_enabled done server_id=%s enabled=%s', server.server_id, next);
        onEnabledChange?.(server.server_id, next);
      });
    },
    [server.server_id, runBusy, onEnabledChange]
  );

  const openReconfigure = useCallback(
    (prefill?: Record<string, string>) => {
      const initial: Record<string, string> = {};
      const initialVisibility: Record<string, boolean> = {};
      for (const key of visibleEnvKeys) {
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
      for (const key of visibleEnvKeys) {
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
            className="w-10 h-10 rounded shrink-0 object-contain bg-surface border border-line-subtle"
          />
        ) : (
          <div className="w-10 h-10 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-lg">
            🔌
          </div>
        )}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h3 className="text-base font-semibold text-content">{server.display_name}</h3>
            <McpStatusBadge status={status} />
          </div>
          {server.description && (
            <p className="text-xs text-content-muted mt-0.5">{server.description}</p>
          )}
          <p className="text-[11px] text-content-faint mt-1 font-mono">{server.qualified_name}</p>
        </div>
      </div>

      {/* Auth required — a graceful, actionable notice rather than the raw HTTP
          401 string. The core reports `unauthorized` (no raw error) for a 401;
          the Connect button below opens the auth modal, which probes the server
          and offers browser sign-in or a token field as appropriate (#3719). */}
      {status === 'unauthorized' && (
        <div className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-4 py-3 text-sm text-amber-700 dark:text-amber-300">
          {t('mcp.detail.authRequired')}
        </div>
      )}

      {/* Error — a genuine (non-auth) connect failure. Suppressed entirely while
          `unauthorized`: the amber notice above is the only message shown, so a
          local action error (e.g. a reconfigure reconnect that re-hits the 401)
          can't re-expose raw transport/auth text in this state (#3719). */}
      {status !== 'unauthorized' && (error || connStatus?.last_error) && (
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
        {/* Connect / Disconnect — hidden when the server is disabled because the
            core refuses connect calls on disabled servers. */}
        {server.enabled &&
          (status !== 'connected' ? (
            <Button
              variant="primary"
              size="sm"
              disabled={busy || status === 'connecting'}
              onClick={handleConnect}>
              {status === 'connecting'
                ? t('mcp.detail.connecting')
                : status === 'unauthorized'
                  ? t('mcp.detail.authenticate')
                  : t('mcp.detail.connect')}
            </Button>
          ) : (
            <Button variant="secondary" size="sm" disabled={busy} onClick={handleDisconnect}>
              {t('mcp.detail.disconnect')}
            </Button>
          ))}

        {/* Enable / Disable toggle */}
        <Button
          variant="secondary"
          size="sm"
          disabled={busy}
          onClick={() => handleSetEnabled(!server.enabled)}>
          {server.enabled ? t('mcp.detail.disable') : t('mcp.detail.enable')}
        </Button>

        {SHOW_CONFIG_ASSISTANT && (
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() => setShowAssistant(prev => !prev)}>
            {showAssistant ? t('mcp.connectAuth.hideHelp') : t('mcp.connectAuth.howToGetToken')}
          </Button>
        )}

        {confirmUninstall ? (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-coral-600 dark:text-coral-400 font-medium">
              {t('mcp.detail.confirmUninstall')}
            </span>
            <Button
              variant="primary"
              tone="danger"
              size="sm"
              disabled={busy}
              onClick={handleUninstall}>
              {t('mcp.detail.confirmUninstallAction')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onClick={() => setConfirmUninstall(false)}>
              {t('common.cancel')}
            </Button>
          </div>
        ) : (
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            disabled={busy}
            onClick={() => setConfirmUninstall(true)}>
            {t('mcp.detail.uninstall')}
          </Button>
        )}
      </div>

      {/* Env keys (names only) + reconfigure affordance */}
      {visibleEnvKeys.length > 0 && (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <p className="text-xs font-medium text-content-secondary">{t('mcp.detail.envVars')}</p>
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
              {visibleEnvKeys.map(key => (
                <span
                  key={key}
                  className="px-2 py-0.5 text-[11px] font-mono rounded bg-surface-subtle text-content-secondary border border-line">
                  {key}
                </span>
              ))}
            </div>
          )}
          {reconfigOpen && (
            <div className="space-y-2 rounded-lg border border-line p-3">
              <p className="text-[11px] text-content-muted">{t('mcp.detail.reconfigureHint')}</p>
              {visibleEnvKeys.map(key => (
                <div key={key} className="space-y-1">
                  <label
                    htmlFor={`reconfig-${key}`}
                    className="block text-[11px] font-medium text-content-secondary">
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
                      className="flex-1 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
                    />
                    <Button
                      variant="secondary"
                      size="xs"
                      onClick={() => setShowReconfig(prev => ({ ...prev, [key]: !prev[key] }))}
                      disabled={busy}
                      className="shrink-0">
                      {showReconfig[key] ? t('mcp.install.hide') : t('mcp.install.show')}
                    </Button>
                  </div>
                </div>
              ))}
              <Button variant="primary" size="sm" disabled={busy} onClick={handleSaveReconfigure}>
                {busy ? t('mcp.detail.reconfigureSaving') : t('mcp.detail.saveReconnect')}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Tool list — only show when connected so stale tools don't linger.
          When connected, each tool gets a "Try" button via `onTryTool`
          that opens the Tool Execution Playground modal below. */}
      <div className="space-y-1">
        <p className="text-xs font-medium text-content-secondary">{t('mcp.detail.tools')}</p>
        <McpToolList
          tools={status === 'connected' ? tools : []}
          onTryTool={status === 'connected' ? setPlaygroundTool : undefined}
        />
      </div>

      {/* Config-help chat (stacked modal). */}
      {SHOW_CONFIG_ASSISTANT && showAssistant && (
        <ConfigHelpModal
          qualifiedName={server.qualified_name}
          displayName={server.display_name}
          description={server.description}
          onClose={() => setShowAssistant(false)}
          onApplySuggestedEnv={handleApplySuggestedEnv}
        />
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

      {/* Upfront auth modal — shown on Connect so the user can add tokens
          (declared required fields and/or custom headers) before dialing. */}
      {connectModalOpen && (
        <ConnectAuthModal
          server={server}
          onClose={() => setConnectModalOpen(false)}
          onConnected={connectedTools => {
            setTools(connectedTools);
            setConnectModalOpen(false);
          }}
        />
      )}
    </div>
  );
};

export default InstalledServerDetail;
