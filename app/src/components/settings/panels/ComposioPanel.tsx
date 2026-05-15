// [composio-direct] Settings panel for the Composio routing mode toggle
// (Backend / Direct BYO API key). Shipped in PR3 of #1710 — see
// `src/openhuman/composio/client.rs::create_composio_client` for the
// matching Rust factory.
//
// Why a separate panel from ComposioTriagePanel:
//   - ComposioTriagePanel governs the per-trigger LLM triage opt-out,
//     a behavior that lives entirely inside the backend-proxied
//     pipeline. Mixing the BYO-key controls into it would conflate two
//     orthogonal concerns and confuse users (triggers don't work at all
//     in direct mode — separately calling that out is cleaner).
//   - PR1 already owns LocalModelPanel and PR2 owns VoicePanel; this PR
//     stays in its lane by introducing a new file rather than editing
//     BackendProviderPanel / LocalModelPanel / VoicePanel.
import { useEffect, useRef, useState } from 'react';

import {
  type ComposioModeStatus,
  openhumanComposioClearApiKey,
  openhumanComposioGetMode,
  openhumanComposioSetApiKey,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

type Mode = 'backend' | 'direct';

const ComposioPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [mode, setMode] = useState<Mode>('backend');
  // Tracks the mode that's actually persisted on disk — distinct from
  // the in-flight `mode` radio selection so we can tell whether a Save
  // click constitutes a Backend → Direct *transition* (which needs a
  // confirmation gate) vs. just persisting a new API key while already
  // in Direct mode.
  const [persistedMode, setPersistedMode] = useState<Mode>('backend');
  const [apiKey, setApiKey] = useState('');
  const [apiKeyStored, setApiKeyStored] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saved' | 'error' | 'cleared'>('idle');
  // Confirmation gate for the Backend → Direct transition. The state
  // machine has two arms: `idle` (Save acts immediately) and
  // `awaiting` (Save was clicked while transitioning Backend → Direct
  // with a fresh key — the user sees the warning copy and must hit
  // "I understand, switch to Direct" or "Cancel"). Direct → Backend
  // doesn't need this because that recovery is reversible (re-paste
  // the key to flip back).
  const [confirmGate, setConfirmGate] = useState<'idle' | 'awaiting'>('idle');
  const saveStatusTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── load current mode status on mount ────────────────────────────
  useEffect(() => {
    let isMounted = true;
    openhumanComposioGetMode()
      .then(res => {
        if (!isMounted) return;
        const status: ComposioModeStatus | undefined = res.result;
        if (!status) return;
        const normalizedMode: Mode = status.mode === 'direct' ? 'direct' : 'backend';
        setMode(normalizedMode);
        setPersistedMode(normalizedMode);
        setApiKeyStored(Boolean(status.api_key_set));
      })
      .catch(err => {
        if (!isMounted) return;
        // [composio-direct] never re-throw — settings panel should
        // still render so the user can recover by toggling manually.
        console.warn('[ComposioPanel] failed to load mode:', err);
      })
      .finally(() => {
        if (isMounted) setLoading(false);
      });

    return () => {
      isMounted = false;
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
      }
    };
  }, []);

  const flashSaved = (status: 'saved' | 'cleared') => {
    setSaveStatus(status);
    if (saveStatusTimer.current !== null) {
      clearTimeout(saveStatusTimer.current);
    }
    saveStatusTimer.current = setTimeout(() => setSaveStatus('idle'), 3000);
    // [composio-cache] Notify in-renderer subscribers (notably
    // useComposioIntegrations) that the routing config just changed so
    // they can drop their cached connection / toolkit state and
    // re-fetch against the new client. Mirrors the core-side
    // DomainEvent::ComposioConfigChanged emitted by the matching RPC
    // op. Without this the integrations panel keeps showing the
    // previous tenant's badge for up to one poll interval (5s).
    try {
      window.dispatchEvent(new CustomEvent('composio:config-changed'));
    } catch (err) {
      // Non-fatal — old browsers without CustomEvent support shouldn't
      // crash the panel.
      console.warn('[composio-cache] dispatch composio:config-changed failed:', err);
    }
  };

  // Indicates this Save click would transition the persisted mode from
  // Backend to Direct *with* a freshly-pasted key. We gate this exact
  // transition on a confirmation step because the consequences are not
  // obvious from the radio toggle alone — the user's previously-linked
  // integrations (Gmail, Slack, GitHub, …) live in TinyHumans' Composio
  // tenant and will simply disappear from the integrations panel until
  // they re-link them through their personal app.composio.dev account.
  const isBackendToDirectTransition = (): boolean => {
    const trimmed = apiKey.trim();
    return persistedMode === 'backend' && mode === 'direct' && trimmed.length > 0;
  };

  const performSave = async () => {
    const trimmed = apiKey.trim();
    setSaving(true);
    try {
      if (mode === 'direct' && trimmed.length > 0) {
        // [composio-direct] persist new key + flip mode to direct.
        await openhumanComposioSetApiKey(trimmed, true);
        // Mask the field after a successful save so the secret is not
        // left dangling in the DOM. The Rust side has the source of
        // truth in the encrypted keychain.
        setApiKey('');
        setApiKeyStored(true);
        setPersistedMode('direct');
        flashSaved('saved');
      } else if (mode === 'backend') {
        // Switching to backend — clear the stored key and reset mode.
        await openhumanComposioClearApiKey();
        setApiKey('');
        setApiKeyStored(false);
        setPersistedMode('backend');
        flashSaved('cleared');
      } else {
        // Direct selected, no new key but one already stored — nothing
        // to persist; just acknowledge.
        flashSaved('saved');
      }
    } catch (err) {
      console.warn('[ComposioPanel] failed to save:', err);
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
        saveStatusTimer.current = null;
      }
      setSaveStatus('error');
    } finally {
      setSaving(false);
      setConfirmGate('idle');
    }
  };

  const handleSave = async () => {
    const trimmed = apiKey.trim();
    if (mode === 'direct' && trimmed.length === 0 && !apiKeyStored) {
      // Direct mode without a key is a no-op — flag it clearly instead
      // of round-tripping to the backend just to get an error string.
      setSaveStatus('error');
      return;
    }
    if (isBackendToDirectTransition()) {
      // [composio-direct] Show the confirmation step instead of saving
      // straight away. The user-visible consequences (existing
      // integrations disappear, triggers don't fire) aren't obvious
      // from the radio toggle alone.
      console.debug('[composio-direct] Backend → Direct transition pending user confirmation');
      setConfirmGate('awaiting');
      return;
    }
    await performSave();
  };

  const handleConfirmTransition = async () => {
    console.debug('[composio-direct] Backend → Direct transition confirmed by user');
    await performSave();
  };

  const handleCancelTransition = () => {
    console.debug('[composio-direct] Backend → Direct transition cancelled by user');
    setConfirmGate('idle');
  };

  if (loading) {
    return (
      <div>
        <SettingsHeader
          title="Composio"
          showBackButton
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
        <div className="p-4">
          <p className="text-sm text-stone-500">Loading…</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <SettingsHeader
        title="Composio"
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-5">
        <p className="text-sm text-stone-500">
          Composio powers integrations with Gmail, Notion, Slack, GitHub, and 1000+ other apps. By
          default OpenHuman proxies these calls through the TinyHumans backend so you don&apos;t
          need to manage an API key. If you prefer to use your own Composio account directly, switch
          to <span className="font-medium">Direct</span> mode and paste your key below.
        </p>

        {/* Mode toggle */}
        <div className="rounded-2xl border border-stone-200 bg-stone-50/60 p-4 space-y-3">
          <fieldset>
            <legend className="text-sm font-medium text-stone-900 mb-2">Routing mode</legend>
            <div className="space-y-2">
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="radio"
                  name="composio-mode"
                  value="backend"
                  checked={mode === 'backend'}
                  onChange={() => setMode('backend')}
                  aria-label="Backend (proxied through TinyHumans)"
                  className="mt-1"
                />
                <div className="text-left">
                  <span className="text-sm font-medium text-stone-900">
                    Backend (proxied through TinyHumans)
                  </span>
                  <p className="text-xs text-stone-500 mt-0.5">
                    Default. Calls go through <span className="font-mono">api.tinyhumans.ai</span>.
                    Trigger webhooks (real-time Gmail / Slack events) work out of the box.
                  </p>
                </div>
              </label>
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="radio"
                  name="composio-mode"
                  value="direct"
                  checked={mode === 'direct'}
                  onChange={() => setMode('direct')}
                  aria-label="Direct (bring your own API key)"
                  className="mt-1"
                />
                <div className="text-left">
                  <span className="text-sm font-medium text-stone-900">
                    Direct (bring your own API key)
                  </span>
                  <p className="text-xs text-stone-500 mt-0.5">
                    Calls go to <span className="font-mono">backend.composio.dev</span> directly.
                    Sovereign / offline-friendly. Tool execution works synchronously; real-time
                    trigger webhooks are <span className="font-medium">not yet routed</span> in
                    direct mode (follow-up issue).
                  </p>
                </div>
              </label>
            </div>
          </fieldset>
        </div>

        {/* API key field — only when Direct is selected */}
        {mode === 'direct' && (
          <div className="space-y-2">
            <label className="block text-sm font-medium text-stone-800" htmlFor="composio-api-key">
              Composio API key
            </label>
            <p className="text-xs text-stone-500">
              Find your key at <span className="font-mono">app.composio.dev/api-keys</span>. The key
              is stored encrypted on this device and is never sent to TinyHumans servers.
            </p>
            <input
              id="composio-api-key"
              type="password"
              autoComplete="off"
              value={apiKey}
              onChange={e => setApiKey(e.target.value)}
              placeholder={
                apiKeyStored
                  ? '••••••••••••  (key stored — paste a new key to replace)'
                  : 'ck_live_xxxxxxxxxxxxxxxx'
              }
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm font-mono text-stone-900 placeholder-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-400"
            />
            {apiKeyStored && (
              <p className="text-xs text-green-600">
                A Composio API key is currently stored on this device.
              </p>
            )}
          </div>
        )}

        {confirmGate === 'awaiting' ? (
          // [composio-direct] Inline confirmation step — kept as a
          // sibling state rather than a portal modal so the warning
          // copy stays in the same scroll context as the toggle the
          // user just changed. Easier to dismiss with the keyboard and
          // composes more naturally with the existing settings panel
          // chrome.
          <div
            role="alertdialog"
            aria-labelledby="composio-confirm-title"
            className="rounded-2xl border border-amber-200 bg-amber-50/80 p-4 space-y-3">
            <p id="composio-confirm-title" className="text-sm font-medium text-amber-900">
              ⚠️ Switching to Direct mode
            </p>
            <div className="text-xs text-amber-900 space-y-2">
              <p>
                Your existing integrations (Gmail, Slack, GitHub, etc. linked through OpenHuman){' '}
                <span className="font-semibold">won&apos;t be visible</span> — they live in
                TinyHumans&apos; Composio tenant.
              </p>
              <p>You&apos;ll need:</p>
              <ol className="list-decimal list-inside space-y-0.5 ml-2">
                <li>
                  An account at <span className="font-mono">app.composio.dev</span> with an API key
                </li>
                <li>To re-link each integration through your personal Composio account</li>
                <li>
                  Note: Composio triggers (real-time webhooks) don&apos;t fire in Direct mode yet —
                  only synchronous tool calls
                </li>
              </ol>
            </div>
            <div className="flex gap-2 pt-1">
              <button
                type="button"
                onClick={handleCancelTransition}
                disabled={saving}
                className="flex-1 py-2 rounded-xl border border-stone-300 bg-white text-stone-800 text-sm font-medium hover:bg-stone-50 transition-colors disabled:opacity-50">
                Cancel
              </button>
              <button
                type="button"
                onClick={handleConfirmTransition}
                disabled={saving}
                className="flex-1 py-2 rounded-xl bg-amber-600 text-white text-sm font-medium hover:bg-amber-500 transition-colors disabled:opacity-50">
                {saving ? 'Switching…' : 'I understand, switch to Direct'}
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            onClick={handleSave}
            disabled={saving}
            className="w-full py-2 rounded-xl bg-primary-600 text-white text-sm font-medium hover:bg-primary-500 transition-colors disabled:opacity-50">
            {saving ? 'Saving…' : 'Save'}
          </button>
        )}

        {saveStatus === 'saved' && (
          <p className="text-xs text-center text-green-600">Settings saved</p>
        )}
        {saveStatus === 'cleared' && (
          <p className="text-xs text-center text-green-600">Switched to Backend mode</p>
        )}
        {saveStatus === 'error' && (
          <p className="text-xs text-center text-red-500">
            Failed to save. Direct mode requires a non-empty API key.
          </p>
        )}
      </div>
    </div>
  );
};

export default ComposioPanel;
