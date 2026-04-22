import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../../../services/coreRpcClient';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

// ---------------------------------------------------------------------------
// Types (mirror GmailSyncStats on the Rust side)
// ---------------------------------------------------------------------------

interface GmailSyncStats {
  account_id: string;
  email: string;
  connected_at_ms: number;
  last_sync_at_ms: number;
  last_sync_count: number;
  cron_job_id: string | null;
}

// ---------------------------------------------------------------------------
// Core RPC helpers
// ---------------------------------------------------------------------------

async function rpcListAccounts(): Promise<GmailSyncStats[]> {
  const result = await callCoreRpc<GmailSyncStats[]>({
    method: 'openhuman.gmail_list_accounts',
    params: {},
  });
  return result ?? [];
}

async function rpcConnectAccount(accountId: string, email: string): Promise<GmailSyncStats> {
  return callCoreRpc<GmailSyncStats>({
    method: 'openhuman.gmail_connect_account',
    params: { account_id: accountId, email },
  });
}

async function rpcDisconnectAccount(accountId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.gmail_disconnect_account', params: { account_id: accountId } });
}

async function rpcSyncNow(accountId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.gmail_sync_now', params: { account_id: accountId } });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatTs(ms: number): string {
  if (ms <= 0) return 'Never';
  return new Date(ms).toLocaleString();
}

// ---------------------------------------------------------------------------
// Account row
// ---------------------------------------------------------------------------

function AccountRow({
  account,
  onSyncNow,
  onDisconnect,
  syncing,
}: {
  account: GmailSyncStats;
  onSyncNow: (accountId: string) => void;
  onDisconnect: (accountId: string) => void;
  syncing: boolean;
}) {
  return (
    <div className="p-4 bg-white border border-stone-200 rounded-xl space-y-3">
      <div className="flex items-start justify-between">
        <div>
          <p className="font-medium text-sm text-stone-900">{account.email}</p>
          <p className="text-xs text-stone-500 mt-0.5">
            Connected {formatTs(account.connected_at_ms)}
          </p>
        </div>
        <span className="px-2 py-1 text-xs font-medium rounded-full border bg-green-50 text-green-700 border-green-200">
          Connected
        </span>
      </div>

      <div className="grid grid-cols-2 gap-2 text-xs text-stone-500">
        <div>
          <span className="font-medium text-stone-700">Last sync</span>
          <p>{formatTs(account.last_sync_at_ms)}</p>
        </div>
        <div>
          <span className="font-medium text-stone-700">Messages ingested</span>
          <p>{account.last_sync_count.toLocaleString()}</p>
        </div>
      </div>

      <div className="flex gap-2">
        <button
          onClick={() => onSyncNow(account.account_id)}
          disabled={syncing}
          className="flex-1 px-3 py-1.5 text-xs font-medium rounded-lg border border-primary-500/30
                     bg-primary-500/10 text-primary-700 hover:bg-primary-500/20 transition-colors
                     disabled:opacity-50 disabled:cursor-not-allowed">
          {syncing ? 'Syncing…' : 'Sync now'}
        </button>
        <button
          onClick={() => onDisconnect(account.account_id)}
          className="px-3 py-1.5 text-xs font-medium rounded-lg border border-red-300
                     bg-red-50 text-red-700 hover:bg-red-100 transition-colors">
          Disconnect
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CDP body event forwarder (listens for MITM bodies, forwards to core)
// ---------------------------------------------------------------------------

// Envelope shape emitted by gmail_scanner::pump_events
interface WebviewEvent {
  account_id: string;
  provider: string;
  kind: string;
  payload: {
    source?: string;
    url?: string;
    body?: string;
    base64_encoded?: boolean;
    [key: string]: unknown;
  };
}

/**
 * Forward a CDP-captured Gmail sync response body to the core RPC handler
 * `openhuman.gmail_ingest_raw_response`. Called for every `webview:event`
 * envelope with provider=gmail and source=cdp-http-body.
 *
 * Errors are logged but not surfaced to the user — ingestion failures are
 * non-fatal and logged server-side.
 */
async function forwardBodyToCore(event: WebviewEvent): Promise<void> {
  const { account_id, payload } = event;
  if (payload.source !== 'cdp-http-body') return;
  const url = payload.url ?? '';
  const body = payload.body ?? '';
  if (!body) return;

  try {
    await callCoreRpc({
      method: 'openhuman.gmail_ingest_raw_response',
      params: { account_id, url, body },
    });
  } catch (e) {
    // Non-fatal: log only.
    if (process.env.NODE_ENV !== 'production') {
      console.debug('[gmail] forwardBodyToCore error:', e);
    }
  }
}

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

const GmailPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [accounts, setAccounts] = useState<GmailSyncStats[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncingId, setSyncingId] = useState<string | null>(null);
  const [connectingEmail, setConnectingEmail] = useState('');
  const [showConnect, setShowConnect] = useState(false);

  // Keep a ref to the unlisten function so we can clean up on unmount.
  const unlistenRef = useRef<(() => void) | null>(null);

  // ---------- data loading ----------

  const loadAccounts = useCallback(async () => {
    try {
      const result = await rpcListAccounts();
      setAccounts(result);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadAccounts();
  }, [loadAccounts]);

  // ---------- CDP body event listener ----------

  useEffect(() => {
    // Subscribe to webview:event envelopes from the scanner.
    // Only runs in Tauri (the listen API is a no-op in a plain browser).
    let cancelled = false;

    listen<WebviewEvent>('webview:event', event => {
      const env = event.payload;
      if (env.provider !== 'gmail') return;
      if (env.payload?.source !== 'cdp-http-body') return;
      if (cancelled) return;
      void forwardBodyToCore(env);
    })
      .then(unlisten => {
        if (cancelled) {
          unlisten();
        } else {
          unlistenRef.current = unlisten;
        }
      })
      .catch(e => {
        if (process.env.NODE_ENV !== 'production') {
          console.debug('[gmail] listen setup error (non-Tauri env?):', e);
        }
      });

    return () => {
      cancelled = true;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  // ---------- actions ----------

  const handleSyncNow = useCallback(
    async (accountId: string) => {
      setSyncingId(accountId);
      try {
        // Signal the core domain that a sync is requested.
        await rpcSyncNow(accountId);
        // Also trigger an IDB backfill scan from the scanner side (best-effort).
        try {
          await invoke('gmail_scanner_backfill', { account_id: accountId });
        } catch {
          // Not available in non-CEF or non-Tauri builds — ignore.
        }
      } catch (e) {
        setError(`Sync failed: ${String(e)}`);
      } finally {
        setSyncingId(null);
        void loadAccounts();
      }
    },
    [loadAccounts]
  );

  const handleDisconnect = useCallback(
    async (accountId: string) => {
      if (
        !window.confirm(
          'Disconnect this Gmail account? Local memory for this account will be deleted.'
        )
      ) {
        return;
      }
      try {
        await rpcDisconnectAccount(accountId);
        // Also purge the webview session (best-effort — may not be open).
        try {
          await invoke('webview_account_purge', { account_id: accountId });
        } catch {
          // Not open — ignore.
        }
        void loadAccounts();
      } catch (e) {
        setError(`Disconnect failed: ${String(e)}`);
      }
    },
    [loadAccounts]
  );

  const handleConnect = useCallback(async () => {
    const email = connectingEmail.trim();
    if (!email) return;

    // Generate a stable account_id from the email.
    const accountId = `gmail_${email.replace(/[^a-z0-9]/gi, '_').toLowerCase()}`;
    try {
      // Open the Gmail webview so the user can log in.
      await invoke('webview_account_open', {
        account_id: accountId,
        provider: 'gmail',
        bounds: { x: 80, y: 80, width: 960, height: 720 },
      });
      // Register the account in the core domain after the webview opens.
      await rpcConnectAccount(accountId, email);
      setConnectingEmail('');
      setShowConnect(false);
      void loadAccounts();
    } catch (e) {
      setError(`Connect failed: ${String(e)}`);
    }
  }, [connectingEmail, loadAccounts]);

  // ---------- render ----------

  return (
    <div>
      <SettingsHeader
        title="Gmail"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Error banner */}
        {error && (
          <div className="p-3 bg-red-50 border border-red-200 rounded-xl text-xs text-red-700">
            {error}
          </div>
        )}

        {/* Account list */}
        {loading ? (
          <div className="py-8 text-center text-sm text-stone-400">Loading…</div>
        ) : accounts.length === 0 ? (
          <div className="py-8 text-center text-sm text-stone-400">
            No Gmail accounts connected yet.
          </div>
        ) : (
          <div className="space-y-3">
            {accounts.map(account => (
              <AccountRow
                key={account.account_id}
                account={account}
                onSyncNow={handleSyncNow}
                onDisconnect={handleDisconnect}
                syncing={syncingId === account.account_id}
              />
            ))}
          </div>
        )}

        {/* Connect new account */}
        {!showConnect ? (
          <button
            onClick={() => setShowConnect(true)}
            className="w-full px-4 py-2.5 text-sm font-medium rounded-xl border border-stone-200
                       bg-white text-stone-700 hover:bg-stone-50 transition-colors">
            + Connect Gmail account
          </button>
        ) : (
          <div className="p-4 bg-white border border-stone-200 rounded-xl space-y-3">
            <p className="text-sm font-medium text-stone-900">Connect Gmail account</p>
            <p className="text-xs text-stone-500">
              Enter your Gmail address then sign in when the browser window opens.
            </p>
            <input
              type="email"
              placeholder="you@gmail.com"
              value={connectingEmail}
              onChange={e => setConnectingEmail(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter') void handleConnect();
              }}
              className="w-full px-3 py-2 text-sm border border-stone-200 rounded-lg
                         focus:outline-none focus:ring-2 focus:ring-primary-500/30"
            />
            <div className="flex gap-2">
              <button
                onClick={() => void handleConnect()}
                disabled={!connectingEmail.trim()}
                className="flex-1 px-4 py-2 text-sm font-medium rounded-lg bg-primary-500 text-white
                           hover:bg-primary-600 transition-colors disabled:opacity-50">
                Open Gmail &amp; Connect
              </button>
              <button
                onClick={() => {
                  setShowConnect(false);
                  setConnectingEmail('');
                }}
                className="px-4 py-2 text-sm font-medium rounded-lg border border-stone-200
                           bg-white text-stone-700 hover:bg-stone-50 transition-colors">
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Privacy notice */}
        <div className="p-4 bg-blue-50 border border-blue-200 rounded-xl">
          <div className="flex items-start space-x-2">
            <svg
              className="w-5 h-5 text-blue-600 flex-shrink-0 mt-0.5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
            <div>
              <p className="font-medium text-blue-700 text-sm">Local-only access</p>
              <p className="text-blue-600 text-xs mt-1">
                Gmail data is read directly from your logged-in browser session. No OAuth tokens or
                credentials are sent to any server. All messages are stored locally in your
                encrypted workspace.
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default GmailPanel;
