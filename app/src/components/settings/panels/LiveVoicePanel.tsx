import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  clearLiveVoiceProviderKey,
  fetchLiveVoiceProviders,
  fetchLiveVoiceSettings,
  type LiveVoiceProvider,
  type LiveVoiceProviders,
  type LiveVoiceSettings,
  type LiveVoiceSettingsPatch,
  saveLiveVoiceProviderKey,
  testLiveVoiceProvider,
  updateLiveVoiceSettings,
} from '../../../services/api/liveVoiceApi';
import { Alert, AlertDescription } from '../../ui/Alert';
import { CenteredLoadingState } from '../../ui/LoadingState';
import StatusLine from '../../ui/StatusLine';
import { toast } from '../../ui/Toast';
import SettingsTabbedPage from '../layout/SettingsTabbedPage';
import LiveVoiceSettingsModal, { type LiveVoiceTestState } from './LiveVoiceSettingsModal';
import LiveVoiceVendorCard from './LiveVoiceVendorCard';
import LiveVoiceVendorLogo from './LiveVoiceVendorLogo';
import { groupVendors, vendorIdOf } from './liveVoiceVendors';

/** A write in flight; its outcome is reported as a toast. */
type Status = { kind: 'idle' } | { kind: 'saving' };

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));

/**
 * Connections → Voice agents: a card per voice service (the core's providers
 * grouped by vendor), marking the one the live voice agent — the mascot's talk
 * mode — runs on and which come with TinyHumans. Each card's Settings modal
 * holds the ways to connect that service (managed or own key), the BYOK key,
 * a connectivity test, and the voice/language choices.
 *
 * Everything is rendered from the core's `voice_live_providers` /
 * `voice_live_settings_get`; every settings write returns the full settings,
 * which replace local state.
 */
const LiveVoicePanel = () => {
  const { t } = useT();
  const [openVendorId, setOpenVendorId] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<LiveVoiceProviders | null>(null);
  const [settings, setSettings] = useState<LiveVoiceSettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'idle' });
  const [keyDrafts, setKeyDrafts] = useState<Record<string, string>>({});
  const [tests, setTests] = useState<Record<string, LiveVoiceTestState>>({});
  const saving = status.kind === 'saving';

  const loadProviders = useCallback(async () => {
    const next = await fetchLiveVoiceProviders();
    setCatalog(next);
    return next;
  }, []);

  useEffect(() => {
    let cancelled = false;
    Promise.all([fetchLiveVoiceProviders(), fetchLiveVoiceSettings()])
      .then(([providers, current]) => {
        if (cancelled) return;
        setCatalog(providers);
        setSettings(current);
      })
      .catch(err => {
        if (!cancelled) setLoadError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const vendors = useMemo(() => (catalog ? groupVendors(catalog.providers) : []), [catalog]);
  const vendorName = (providerId: string) =>
    vendors.find(v => v.id === vendorIdOf(providerId))?.name ?? providerId;

  const logo = (providerId: string) => (
    <LiveVoiceVendorLogo vendorId={vendorIdOf(providerId)} className="h-4.5 w-4.5" />
  );

  const succeed = (options: Parameters<typeof toast.add>[0]) => {
    toast.add({ type: 'success', ...options });
    setStatus({ kind: 'idle' });
  };

  const fail = (err: unknown) => {
    toast.add({
      type: 'error',
      title: t('connections.voiceAgents.toastSaveFailed'),
      description: errorMessage(err),
    });
    setStatus({ kind: 'idle' });
  };

  const persist = async (patch: LiveVoiceSettingsPatch) => {
    setStatus({ kind: 'saving' });
    try {
      const next = await updateLiveVoiceSettings(patch);
      setSettings(next);
      const switchedTo = patch.default_provider;
      if (switchedTo) {
        setCatalog(prev => (prev ? { ...prev, default_provider: switchedTo } : prev));
        succeed({
          title: t('connections.voiceAgents.toastSwitched'),
          description: t('connections.voiceAgents.toastSwitchedBody').replace(
            '{name}',
            vendorName(switchedTo)
          ),
          data: { icon: logo(switchedTo) },
        });
      } else {
        succeed({ title: t('connections.voiceAgents.toastVoiceSaved') });
      }
    } catch (err) {
      fail(err);
    }
  };

  const saveKey = async (provider: LiveVoiceProvider) => {
    const draft = keyDrafts[provider.id]?.trim();
    if (!draft || !provider.key_slug) return;
    setStatus({ kind: 'saving' });
    try {
      await saveLiveVoiceProviderKey(provider.key_slug, draft);
      setKeyDrafts(prev => ({ ...prev, [provider.id]: '' }));
      await loadProviders();
      succeed({
        title: t('connections.voiceAgents.toastKeySaved'),
        description: t('connections.voiceAgents.toastKeySavedBody').replace(
          '{name}',
          vendorName(provider.id)
        ),
        data: { icon: logo(provider.id) },
      });
    } catch (err) {
      fail(err);
    }
  };

  const clearKey = async (provider: LiveVoiceProvider) => {
    if (!provider.key_slug) return;
    setStatus({ kind: 'saving' });
    try {
      await clearLiveVoiceProviderKey(provider.key_slug);
      await loadProviders();
      succeed({ title: t('connections.voiceAgents.toastKeyRemoved') });
    } catch (err) {
      fail(err);
    }
  };

  const runTest = async (providerId: string) => {
    setTests(prev => ({ ...prev, [providerId]: { kind: 'testing' } }));
    try {
      const result = await testLiveVoiceProvider(providerId);
      setTests(prev => ({ ...prev, [providerId]: { kind: 'done', result } }));
    } catch (err) {
      setTests(prev => ({
        ...prev,
        [providerId]: {
          kind: 'done',
          result: { ok: false, latency_ms: null, error: errorMessage(err) },
        },
      }));
    }
  };

  const defaultProvider = settings?.default_provider || catalog?.default_provider || '';

  const openVendor = vendors.find(v => v.id === openVendorId) ?? null;

  const statusLine = (
    <StatusLine saving={saving} savingLabel={t('connections.voiceAgents.saving')} />
  );

  return (
    <SettingsTabbedPage
      title={t('connections.tabs.voiceAgents')}
      description={t('connections.header.voiceAgents')}>
      <div className="@container flex flex-col gap-4" data-testid="live-voice-panel">
        {loadError && (
          <Alert variant="destructive">
            <AlertDescription>
              {t('connections.voiceAgents.loadFailed')}: {loadError}
            </AlertDescription>
          </Alert>
        )}
        {!catalog && !loadError && <CenteredLoadingState label={t('common.loading')} />}

        {catalog && (
          <div
            className="grid items-stretch gap-3 @xl:grid-cols-2 @4xl:grid-cols-3"
            data-testid="live-voice-providers">
            {vendors.map(vendor => (
              <LiveVoiceVendorCard
                key={vendor.id}
                vendor={vendor}
                defaultProvider={defaultProvider}
                saving={saving}
                onUse={providerId => void persist({ default_provider: providerId })}
                onOpenSettings={() => setOpenVendorId(vendor.id)}
              />
            ))}
          </div>
        )}

        {!openVendor && statusLine}
      </div>

      {openVendor && (
        <LiveVoiceSettingsModal
          vendor={openVendor}
          defaultProvider={defaultProvider}
          settings={settings}
          saving={saving}
          tests={tests}
          keyDrafts={keyDrafts}
          status={statusLine}
          onClose={() => setOpenVendorId(null)}
          onUse={providerId => void persist({ default_provider: providerId })}
          onTest={providerId => void runTest(providerId)}
          onKeyDraft={(providerId, value) =>
            setKeyDrafts(prev => ({ ...prev, [providerId]: value }))
          }
          onSaveKey={provider => void saveKey(provider)}
          onClearKey={provider => void clearKey(provider)}
          onPersist={patch => void persist(patch)}
        />
      )}
    </SettingsTabbedPage>
  );
};

export default LiveVoicePanel;
