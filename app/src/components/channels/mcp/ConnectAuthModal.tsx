/**
 * ConnectAuthModal — opened when the user clicks "Connect" on an installed MCP
 * server. Lets the user supply auth before connecting:
 *
 *   • Known fields — keys the registry declares as required (`required_env_keys`,
 *     e.g. an `Authorization` header for an HTTP-remote server) plus any keys
 *     already stored on the install. Rendered as secret inputs.
 *   • Custom headers — free-form name/value rows for servers whose registry
 *     metadata declares no auth (mislabelled remotes like inference.sh) where
 *     the user nonetheless has a token to paste (e.g. `Authorization: Bearer …`).
 *
 * On submit, non-empty values are persisted via `update_env` (which stores them
 * encrypted and reconnects); for HTTP-remote installs each entry becomes a
 * request header on connect (see core `build_http_auth`). When the user supplies
 * nothing, we just connect — open servers need no auth.
 *
 * This is the upfront (non-reactive) auth step: it always appears on Connect so
 * the user can set credentials before the first attempt, rather than only after
 * a 401.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import { openUrl } from '../../../utils/openUrl';
import ConfigHelpModal from './ConfigHelpModal';
import type { InstalledServer, McpTool } from './types';

const log = debug('mcp-clients:connect-auth');

interface ConnectAuthModalProps {
  server: InstalledServer;
  onClose: () => void;
  /** Called with the connected server's tools once connect succeeds. */
  onConnected: (tools: McpTool[]) => void;
}

interface CustomHeader {
  id: number;
  name: string;
  value: string;
  /** `bearer` prepends `Bearer ` to the value (the common case); `raw` sends
   * the value verbatim (for API-key headers or other schemes). */
  scheme: 'bearer' | 'raw';
}

/** Apply a header scheme to a value: `bearer` prepends `Bearer ` unless the
 * value already carries a scheme; `raw` is verbatim. */
const applyScheme = (scheme: 'bearer' | 'raw', value: string): string => {
  const v = value.trim();
  if (!v) return v;
  if (scheme === 'bearer' && !/^bearer\s/i.test(v)) return `Bearer ${v}`;
  return v;
};

const ConnectAuthModal = ({ server, onClose, onConnected }: ConnectAuthModalProps) => {
  const { t } = useT();
  // Declared/known keys: union of the registry's required keys and any keys
  // already on the install. Seeded from the install immediately; refined by a
  // best-effort registry_get (which also carries newly-declared headers).
  // `__`-prefixed keys are internal bookkeeping (OAuth refresh bundle) — never
  // render them as input fields.
  const [knownKeys, setKnownKeys] = useState<string[]>(
    server.env_keys.filter(k => !k.startsWith('__'))
  );
  const [values, setValues] = useState<Record<string, string>>({});
  const [reveal, setReveal] = useState<Record<string, boolean>>({});
  // Per-declared-field scheme (the registry doesn't say Bearer vs raw — only a
  // free-text description — so the user picks). Defaults to Bearer for an
  // `Authorization` header (the overwhelming case), raw for anything else.
  const [knownSchemes, setKnownSchemes] = useState<Record<string, 'bearer' | 'raw'>>({});
  // Memoized on `knownSchemes` so callbacks that depend on it (e.g.
  // `handleConnect`) re-derive the right scheme after the user flips the
  // dropdown — a stale closure here would send the wrong prefix on submit.
  const schemeFor = useCallback(
    (key: string): 'bearer' | 'raw' =>
      knownSchemes[key] ?? (key.toLowerCase() === 'authorization' ? 'bearer' : 'raw'),
    [knownSchemes]
  );
  const [customHeaders, setCustomHeaders] = useState<CustomHeader[]>([]);
  const [nextId, setNextId] = useState(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Detected auth style: drives whether we show "Sign in" (browser OAuth) vs.
  // the token/header fields. `detecting` until the probe returns.
  const [authKind, setAuthKind] = useState<'detecting' | 'none' | 'token' | 'oauth'>('detecting');
  const [oauthWaiting, setOauthWaiting] = useState(false);
  // Opens the stacked configuration-help chat modal.
  const [showConfigHelp, setShowConfigHelp] = useState(false);

  // Probe how this server authenticates so we render the right control. The
  // registry can't tell us (it mislabels), so we ask the server.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const d = await mcpClientsApi.detectAuth(server.server_id);
        if (!cancelled) setAuthKind(d.kind);
      } catch (err) {
        log('detect_auth failed (non-fatal): %s', err instanceof Error ? err.message : err);
        if (!cancelled) setAuthKind('token');
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [server.server_id]);

  // Browser-OAuth: begin (discover + DCR + PKCE), open the authorize URL, then
  // poll until the /oauth/mcp/callback route has stored the token + reconnected.
  const handleOAuth = useCallback(() => {
    setBusy(true);
    setError(null);
    setOauthWaiting(true);
    void (async () => {
      try {
        const url = await mcpClientsApi.oauthBegin(server.server_id);
        await openUrl(url);
        const started = Date.now();
        const poll = async (): Promise<void> => {
          const statuses = await mcpClientsApi.status();
          const mine = statuses.find(s => s.server_id === server.server_id);
          if (mine?.status === 'connected') {
            const result = await mcpClientsApi.connect(server.server_id);
            onConnected(result.tools ?? []);
            onClose();
            return;
          }
          if (Date.now() - started > 180000) {
            throw new Error(t('mcp.connectAuth.oauthTimeout'));
          }
          window.setTimeout(() => {
            void poll().catch(handlePollError);
          }, 2500);
        };
        const handlePollError = (err: unknown) => {
          setError(err instanceof Error ? err.message : String(err));
          setOauthWaiting(false);
          setBusy(false);
        };
        await poll().catch(handlePollError);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('oauth failed: %s', msg);
        setError(msg);
        setOauthWaiting(false);
        setBusy(false);
      }
    })();
  }, [server.server_id, onConnected, onClose, t]);

  // Best-effort: pull the registry's declared required keys so a server that
  // *labels* its auth (e.g. `Authorization`) shows that field even if it was
  // installed with no env. Network failures are non-fatal — we fall back to the
  // install's own env_keys and the custom-header rows.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const detail = await mcpClientsApi.registryGet(server.qualified_name);
        if (cancelled) return;
        const declared = detail.required_env_keys ?? [];
        setKnownKeys(prev => Array.from(new Set([...prev, ...declared])));
      } catch (err) {
        log('registry_get failed (non-fatal): %s', err instanceof Error ? err.message : err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [server.qualified_name]);

  // Seed a blank custom-header row when the server declares nothing, so the
  // user has an obvious place to paste a token (the mislabelled-remote case).
  useEffect(() => {
    if (knownKeys.length === 0 && customHeaders.length === 0) {
      setCustomHeaders([{ id: 0, name: 'Authorization', value: '', scheme: 'bearer' }]);
    }
  }, [knownKeys.length, customHeaders.length]);

  const addCustomHeader = useCallback(() => {
    setCustomHeaders(prev => [...prev, { id: nextId, name: '', value: '', scheme: 'bearer' }]);
    setNextId(n => n + 1);
  }, [nextId]);

  const removeCustomHeader = useCallback((id: number) => {
    setCustomHeaders(prev => prev.filter(h => h.id !== id));
  }, []);

  const handleConnect = useCallback(() => {
    setBusy(true);
    setError(null);
    void (async () => {
      try {
        // Build the env/header map: declared values + named custom headers,
        // skipping blanks so we never store empty keys.
        const env: Record<string, string> = {};
        for (const key of knownKeys) {
          const v = values[key]?.trim();
          if (v) env[key] = applyScheme(schemeFor(key), v);
        }
        for (const h of customHeaders) {
          const name = h.name.trim();
          const value = applyScheme(h.scheme, h.value);
          if (name && value) env[name] = value;
        }

        let tools: McpTool[] = [];
        if (Object.keys(env).length > 0) {
          log('connect-with-auth server_id=%s keys=%o', server.server_id, Object.keys(env));
          const result = await mcpClientsApi.updateEnv({ server_id: server.server_id, env });
          if (result.status !== 'connected') {
            throw new Error(result.error ?? t('mcp.connectAuth.reconnectFailed'));
          }
          tools = result.tools ?? [];
        } else {
          log('connect (no auth supplied) server_id=%s', server.server_id);
          const result = await mcpClientsApi.connect(server.server_id);
          tools = result.tools ?? [];
        }
        onConnected(tools);
        onClose();
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('connect failed: %s', msg);
        setError(msg);
      } finally {
        setBusy(false);
      }
    })();
  }, [knownKeys, values, customHeaders, schemeFor, server.server_id, onConnected, onClose, t]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('mcp.connectAuth.title').replace('{name}', server.display_name)}
      onMouseDown={e => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4 py-6 overflow-y-auto">
      <div className="w-full max-w-md rounded-xl bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 shadow-xl p-5 space-y-4">
        <div>
          <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
            {t('mcp.connectAuth.title').replace('{name}', server.display_name)}
          </h3>
          <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
            {t('mcp.connectAuth.hint')}
          </p>
          <button
            type="button"
            onClick={() => setShowConfigHelp(true)}
            className="mt-1 text-[11px] font-medium text-primary-600 dark:text-primary-400 hover:underline">
            {t('mcp.connectAuth.howToGetToken')}
          </button>
        </div>

        {error && (
          <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300 break-words">
            {error}
          </div>
        )}

        {/* Browser OAuth — shown when detection says this server needs a sign-in. */}
        {authKind === 'oauth' && (
          <div className="space-y-2 rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 p-3">
            <p className="text-xs text-stone-600 dark:text-neutral-300">
              {t('mcp.connectAuth.oauthHint')}
            </p>
            <button
              type="button"
              onClick={handleOAuth}
              disabled={busy}
              className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
              {oauthWaiting ? t('mcp.connectAuth.oauthWaiting') : t('mcp.connectAuth.signIn')}
            </button>
            <p className="text-[11px] text-stone-400 dark:text-neutral-500">
              {t('mcp.connectAuth.oauthOrToken')}
            </p>
          </div>
        )}

        {/* Declared / known fields */}
        {knownKeys.length > 0 && (
          <div className="space-y-2">
            <p className="text-[11px] font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              {t('mcp.connectAuth.requiredLabel')}
            </p>
            {knownKeys.map(key => (
              <div key={key} className="space-y-1">
                <label
                  htmlFor={`auth-${key}`}
                  className="block text-[11px] font-medium text-stone-600 dark:text-neutral-400 font-mono">
                  {key}
                </label>
                <div className="flex gap-2">
                  <select
                    value={schemeFor(key)}
                    onChange={e =>
                      setKnownSchemes(prev => ({
                        ...prev,
                        [key]: e.target.value as 'bearer' | 'raw',
                      }))
                    }
                    disabled={busy}
                    title={t('mcp.connectAuth.schemeLabel')}
                    className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-1.5 py-1.5 text-[11px] text-stone-700 dark:text-neutral-200 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50">
                    <option value="bearer">{t('mcp.connectAuth.schemeBearer')}</option>
                    <option value="raw">{t('mcp.connectAuth.schemeRaw')}</option>
                  </select>
                  <input
                    id={`auth-${key}`}
                    type={reveal[key] ? 'text' : 'password'}
                    value={values[key] ?? ''}
                    onChange={e => setValues(prev => ({ ...prev, [key]: e.target.value }))}
                    placeholder={t('mcp.install.enterValue').replace('{key}', key)}
                    disabled={busy}
                    // Suppress Chromium password-manager autofill so a token saved
                    // for one MCP doesn't pre-fill another's field.
                    autoComplete="new-password"
                    data-1p-ignore
                    data-lpignore="true"
                    data-form-type="other"
                    className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
                  />
                  <button
                    type="button"
                    onClick={() => setReveal(prev => ({ ...prev, [key]: !prev[key] }))}
                    disabled={busy}
                    className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-2 py-1 text-[11px] text-stone-500 dark:text-neutral-400 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
                    {reveal[key] ? t('mcp.install.hide') : t('mcp.install.show')}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Custom headers */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <p className="text-[11px] font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              {t('mcp.connectAuth.customHeadersLabel')}
            </p>
            <button
              type="button"
              onClick={addCustomHeader}
              disabled={busy}
              className="text-[11px] font-medium text-primary-600 dark:text-primary-400 hover:underline disabled:opacity-50">
              {t('mcp.connectAuth.addHeader')}
            </button>
          </div>
          {customHeaders.length === 0 && (
            <p className="text-[11px] text-stone-400 dark:text-neutral-500">
              {t('mcp.connectAuth.customHeadersEmpty')}
            </p>
          )}
          {customHeaders.map(h => (
            <div
              key={h.id}
              className="space-y-1.5 rounded-lg border border-stone-200 dark:border-neutral-800 p-2">
              {/* Row 1: header name + scheme + remove */}
              <div className="flex gap-2">
                <input
                  value={h.name}
                  onChange={e =>
                    setCustomHeaders(prev =>
                      prev.map(x => (x.id === h.id ? { ...x, name: e.target.value } : x))
                    )
                  }
                  placeholder={t('mcp.connectAuth.headerName')}
                  disabled={busy}
                  autoComplete="off"
                  data-1p-ignore
                  data-lpignore="true"
                  data-form-type="other"
                  className="flex-1 min-w-0 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs font-mono text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
                />
                <select
                  value={h.scheme}
                  onChange={e =>
                    setCustomHeaders(prev =>
                      prev.map(x =>
                        x.id === h.id ? { ...x, scheme: e.target.value as 'bearer' | 'raw' } : x
                      )
                    )
                  }
                  disabled={busy}
                  title={t('mcp.connectAuth.schemeLabel')}
                  className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-1.5 py-1.5 text-[11px] text-stone-700 dark:text-neutral-200 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50">
                  <option value="bearer">{t('mcp.connectAuth.schemeBearer')}</option>
                  <option value="raw">{t('mcp.connectAuth.schemeRaw')}</option>
                </select>
                <button
                  type="button"
                  onClick={() => removeCustomHeader(h.id)}
                  disabled={busy}
                  aria-label={t('mcp.connectAuth.removeHeader')}
                  className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-2 py-1 text-[11px] text-stone-500 dark:text-neutral-400 hover:border-coral-300 hover:text-coral-600 disabled:opacity-50">
                  ✕
                </button>
              </div>
              {/* Row 2: full-width value (tokens are long) */}
              <input
                type="password"
                value={h.value}
                onChange={e =>
                  setCustomHeaders(prev =>
                    prev.map(x => (x.id === h.id ? { ...x, value: e.target.value } : x))
                  )
                }
                placeholder={t('mcp.connectAuth.headerValue')}
                disabled={busy}
                // Suppress Chromium password-manager autofill (token leakage
                // across MCP servers on the shared app origin).
                autoComplete="new-password"
                data-1p-ignore
                data-lpignore="true"
                data-form-type="other"
                className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
              />
            </div>
          ))}
        </div>

        {/* Actions */}
        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
            {t('common.cancel')}
          </button>
          <button
            type="button"
            onClick={handleConnect}
            disabled={busy}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
            {busy ? t('mcp.detail.connecting') : t('mcp.detail.connect')}
          </button>
        </div>
      </div>

      {/* Stacked configuration-help chat modal (above this one). */}
      {showConfigHelp && (
        <ConfigHelpModal
          qualifiedName={server.qualified_name}
          displayName={server.display_name}
          description={server.description}
          onClose={() => setShowConfigHelp(false)}
        />
      )}
    </div>
  );
};

export default ConnectAuthModal;
