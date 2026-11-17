/**
 * Memory → Provider: memory runs on CortexDB, shown as one CortexDB card with a
 * chip per way to reach it (like Gemini on Connections → Voice agents):
 *
 * - TinyHumans (`builtin`, the default): the `tinyhumans` engine, the
 *   TinyHumans backend's `/memory/*` API (CortexDB hosted per account),
 *   authenticated by sign-in. Free: Basic includes 1 GB of memory and Pro
 *   20 GB, memory inference is never charged, and the fair-use terms the
 *   panel states apply. Signed out (or on a local
 *   session) it cannot be selected.
 * - Your API key (`apikey`): the `cortexdb` engine on CortexDB's managed API.
 *   The endpoint is fixed; only the key is entered.
 * - Local (`selfhost`): the `cortexdb` engine on a server on this computer.
 *   Local is loopback only (a product rule); either scheme is fine there. The
 *   core itself allows https to any host and cleartext http only to loopback,
 *   so this check is the stricter of the two.
 *
 * TinyHumans connects inline. Your API key and Local open their form in a
 * modal; the chip becomes the selected one only once that connection saves.
 * A configured key or Local connection shows one line on the card with Edit,
 * which reopens the modal.
 *
 * Which chip is in use is derived from `memory_engine_get`: `tinyhumans` is
 * TinyHumans, and `cortexdb` is Local when its endpoint is loopback, else your
 * API key. `none` is the Disabled card beside it: memory turned off on purpose,
 * with every connection's settings kept. Engines that are not supported yet are listed as coming soon.
 *
 * debug logging: DEBUG=openhuman:memory:engine
 */
import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import {
  type EngineSetRequest,
  type EngineState,
  isMemoryDisabled,
  isMemoryOn,
  MEMORY_DISABLED_ENGINE,
  memoryEngineSet,
  memoryErrorMessage,
} from '../../services/api/memoryApi';
import { isLocalSessionToken } from '../../utils/localSession';
import { Alert, AlertDescription, AlertTitle, Button } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import { ModalShell } from '../ui/ModalShell';
import { toast } from '../ui/Toast';
import MemoryComingSoon from './MemoryComingSoon';
import MemoryConnectionPanel from './MemoryConnectionPanel';
import MemoryCortexAnnouncement from './MemoryCortexAnnouncement';
import MemoryCortexCard from './MemoryCortexCard';
import MemoryDisabledCard from './MemoryDisabledCard';
import MemoryProviderLogo, { type MemoryProviderOption } from './MemoryProviderLogo';

export { CORTEXDB_SELF_HOST_DOCS_URL } from './MemoryConnectionPanel';

const log = debug('openhuman:memory:engine');

type EngineOption = MemoryProviderOption;

/**
 * True for an http(s) URL whose host is this computer (localhost, 127.x, ::1).
 * Both schemes are accepted: the core refuses only cleartext http off loopback.
 */
export function isLoopbackEndpoint(raw: string): boolean {
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    return false;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
  const host = url.hostname;
  return host === 'localhost' || host === '[::1]' || /^127(\.\d{1,3}){3}$/.test(host);
}

/** The provider the configured engine corresponds to. */
function optionOf(state: EngineState | null): EngineOption | null {
  if (state?.engine === 'tinyhumans') return 'builtin';
  if (state?.engine === 'cortexdb') {
    return state.endpoint && isLoopbackEndpoint(state.endpoint) ? 'selfhost' : 'apikey';
  }
  return null;
}

interface MemoryEngineTabProps {
  /** The current engine state (null while the page is still loading it). */
  state: EngineState | null;
  /** Called with the new state after a successful switch. */
  onStateChange: (state: EngineState) => void;
  /** Render without the outer spacing (onboarding embeds this tab). */
  embedded?: boolean;
}

export default function MemoryEngineTab({ state, onStateChange, embedded }: MemoryEngineTabProps) {
  const { t } = useT();
  const { snapshot } = useCoreState();
  const signedIn = snapshot.auth.isAuthenticated && !isLocalSessionToken(snapshot.sessionToken);
  const plan = snapshot.currentUser?.subscription?.plan ?? null;

  const active = optionOf(state);
  const on = isMemoryOn(state);

  // The chip the user picked; until then, the configured connection, else
  // TinyHumans (the free, zero-setup default). Only TinyHumans is picked
  // directly: the others are picked by connecting them in their modal.
  const [picked, setPicked] = useState<EngineOption | null>(null);
  const selected: EngineOption = picked ?? active ?? 'builtin';
  // The key or Local connection whose modal is open.
  const [dialog, setDialog] = useState<Exclude<EngineOption, 'builtin'> | null>(null);
  const [saving, setSaving] = useState<EngineOption | 'disabled' | null>(null);
  const [errors, setErrors] = useState<Partial<Record<EngineOption, string>>>({});
  const [cloudKey, setCloudKey] = useState('');
  // Untouched (null) shows the configured local endpoint, which may arrive
  // after the first render when a host loads the state itself.
  const [typedEndpoint, setTypedEndpoint] = useState<string | null>(null);
  const [localKey, setLocalKey] = useState('');

  const titles: Record<EngineOption, string> = {
    builtin: t('memoryPage.engine.builtin.title'),
    apikey: t('memoryPage.engine.apiKeyOption.title'),
    selfhost: t('memoryPage.engine.selfHost.title'),
  };

  const select = useCallback(
    async (option: EngineOption, req: EngineSetRequest): Promise<boolean> => {
      const wasActive = optionOf(state) === option;
      setSaving(option);
      setErrors(prev => ({ ...prev, [option]: undefined }));
      try {
        const next = await memoryEngineSet(req);
        log('engine set (%s): %s status=%s', option, next.engine ?? 'none', next.status);
        onStateChange(next);
        toast.add(
          wasActive
            ? { type: 'success', title: t('memoryPage.engine.toastSaved') }
            : {
                type: 'success',
                title: t('memoryPage.engine.toastSwitched'),
                description: t('memoryPage.engine.toastSwitchedBody').replace(
                  '{name}',
                  titles[option]
                ),
                data: { icon: <MemoryProviderLogo option={option} className="h-5 w-5" /> },
              }
        );
        return true;
      } catch (err) {
        log('engine set (%s) failed: %o', option, err);
        setErrors(prev => ({ ...prev, [option]: memoryErrorMessage(err, t) }));
        return false;
      } finally {
        setSaving(null);
      }
    },
    // `titles` is rebuilt from `t` every render; `t` is the real dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [onStateChange, state, t]
  );

  if (!state) {
    return <CenteredLoadingState label={t('memoryPage.loading')} />;
  }

  const disabled = isMemoryDisabled(state);

  // Disabled: memory off on purpose. Every connection's settings are kept,
  // so picking a provider again turns it back on.
  const disable = async () => {
    setSaving('disabled');
    try {
      const next = await memoryEngineSet({ engine: MEMORY_DISABLED_ENGINE });
      log('memory disabled: status=%s', next.status);
      onStateChange(next);
      setPicked(null);
      toast.add({ type: 'success', title: t('memoryPage.engine.disabled.toast') });
    } catch (err) {
      log('disable failed: %o', err);
      toast.add({
        type: 'error',
        title: t('memoryPage.engine.disabled.toastFailed'),
        description: memoryErrorMessage(err, t),
      });
    } finally {
      setSaving(null);
    }
  };

  // Off: a plain prompt to pick a provider. The core's reason is developer
  // text ("legacy memory backend is unsupported…"), so it is not shown here;
  // degraded and down keep theirs, which names what is wrong.
  const statusBanner = (() => {
    if (disabled) {
      return (
        <Alert variant="info" data-testid="memory-engine-status-disabled">
          <AlertTitle>{t('memoryPage.disabled.title')}</AlertTitle>
          <AlertDescription>{t('memoryPage.engine.disabled.banner')}</AlertDescription>
        </Alert>
      );
    }
    if (!on) {
      return (
        <Alert variant="info" data-testid="memory-engine-status-off">
          <AlertTitle>{t('memoryPage.off.title')}</AlertTitle>
          <AlertDescription>{t('memoryPage.engine.offPrompt')}</AlertDescription>
        </Alert>
      );
    }
    if (state.status === 'degraded' || state.status === 'down') {
      return (
        <Alert
          variant={state.status === 'down' ? 'destructive' : 'warning'}
          data-testid={`memory-engine-status-${state.status}`}>
          <AlertTitle>
            {state.status === 'down'
              ? t('memoryPage.engine.statusDown')
              : t('memoryPage.engine.statusDegraded')}
          </AlertTitle>
          {state.reason ? <AlertDescription>{state.reason}</AlertDescription> : null}
        </Alert>
      );
    }
    return null;
  })();

  const status = (() => {
    if (!active) return null;
    if (!on) return { variant: 'warning' as const, label: t('memoryPage.engine.statusOff') };
    if (state.status === 'down') {
      return { variant: 'danger' as const, label: t('memoryPage.engine.badgeDown') };
    }
    if (state.status === 'degraded') {
      return { variant: 'warning' as const, label: t('memoryPage.engine.badgeDegraded') };
    }
    return { variant: 'primary' as const, label: t('memoryPage.engine.inUse') };
  })();

  // TinyHumans: one click when signed in.
  const useBuiltin = () => void select('builtin', { engine: 'tinyhumans' });

  // Cloud: the endpoint is CortexDB's managed API. Sending a blank endpoint
  // clears any custom (local) endpoint, so the engine falls back to it.
  const cloudKeyRequired = !(active === 'apikey' && state.has_key);
  const canSubmitCloud = saving === null && (!cloudKeyRequired || cloudKey.trim().length > 0);
  const submitCloud = async () => {
    if (!canSubmitCloud) return;
    const req: EngineSetRequest = { engine: 'cortexdb', endpoint: '' };
    if (cloudKey.trim()) req.api_key = cloudKey.trim();
    if (await select('apikey', req)) {
      setCloudKey('');
      setPicked(null);
      setDialog(null);
    }
  };

  // Local: loopback only.
  const localEndpoint = typedEndpoint ?? (active === 'selfhost' ? (state.endpoint ?? '') : '');
  const endpointTyped = localEndpoint.trim().length > 0;
  const endpointLocal = isLoopbackEndpoint(localEndpoint);
  const localKeyRequired = !(active === 'selfhost' && state.has_key);
  const canSubmitLocal =
    saving === null && endpointLocal && (!localKeyRequired || localKey.trim().length > 0);
  const submitLocal = async () => {
    if (!canSubmitLocal) return;
    const req: EngineSetRequest = { engine: 'cortexdb', endpoint: localEndpoint.trim() };
    if (localKey.trim()) req.api_key = localKey.trim();
    if (await select('selfhost', req)) {
      setLocalKey('');
      setPicked(null);
      setDialog(null);
    }
  };

  const onSelectChip = (option: EngineOption) => {
    log('chip: %s', option);
    if (option === 'builtin') {
      setPicked('builtin');
      setDialog(null);
      return;
    }
    setErrors(prev => ({ ...prev, [option]: undefined }));
    setDialog(option);
  };

  const panel = (option: EngineOption) => (
    <MemoryConnectionPanel
      key={option}
      option={option}
      state={state}
      active={option === active}
      signedIn={signedIn}
      plan={plan}
      saving={saving === 'disabled' ? null : saving}
      error={errors[option]}
      cloudKey={cloudKey}
      onCloudKey={setCloudKey}
      localEndpoint={localEndpoint}
      onLocalEndpoint={setTypedEndpoint}
      endpointInvalid={endpointTyped && !endpointLocal}
      localKey={localKey}
      onLocalKey={setLocalKey}
      canSubmit={
        option === 'builtin'
          ? saving === null && signedIn
          : option === 'apikey'
            ? canSubmitCloud
            : canSubmitLocal
      }
      onSubmit={
        option === 'builtin'
          ? useBuiltin
          : option === 'apikey'
            ? () => void submitCloud()
            : () => void submitLocal()
      }
    />
  );

  // The card's body for a configured key or Local connection: one line and Edit.
  const connectedLine = (option: Exclude<EngineOption, 'builtin'>) => (
    <div
      className="flex items-center justify-between gap-2"
      role="tabpanel"
      data-testid={`memory-engine-connected-${option}`}>
      <p className="min-w-0 truncate text-xs text-content-secondary">
        {option === 'apikey'
          ? t('memoryPage.engine.apiKeyOption.connected')
          : t('memoryPage.engine.selfHost.connected').replace('{endpoint}', state.endpoint ?? '')}
      </p>
      <Button
        size="xs"
        variant="secondary"
        analyticsId={`memory-engine-${option}-edit`}
        data-testid={`memory-engine-${option}-edit`}
        onClick={() => onSelectChip(option)}>
        {t('common.edit')}
      </Button>
    </div>
  );

  return (
    <div
      className={`@container ${embedded ? 'space-y-5' : 'w-full space-y-6 animate-fade-up'}`}
      data-testid="memory-engine-tab">
      {/* Onboarding embeds this tab; the announcement is for Memory → Provider only.
          Keyed by user so the per-user dismissal is re-read on an account switch. */}
      {!embedded && <MemoryCortexAnnouncement key={snapshot.auth.userId ?? 'signed-out'} />}
      {statusBanner}

      {/* Sized like one cell of the coming-soon grid below, not the full width.
          Onboarding embeds this tab in a narrow column, where it fills it. */}
      <div className={embedded ? undefined : 'grid gap-2.5 @md:grid-cols-2 @3xl:grid-cols-3'}>
        <MemoryCortexCard
          selected={selected}
          onSelect={onSelectChip}
          active={active}
          status={status}>
          {selected === 'builtin' ? panel('builtin') : connectedLine(selected)}
        </MemoryCortexCard>
        {/* Onboarding embeds this tab to pick a provider, not to opt out. */}
        {!embedded && (
          <MemoryDisabledCard
            active={disabled}
            saving={saving !== null}
            onDisable={() => void disable()}
          />
        )}
      </div>

      {dialog && (
        <ModalShell
          title={titles[dialog]}
          titleId={`memory-engine-${dialog}-dialog-title`}
          icon={<MemoryProviderLogo option={dialog} className="h-5 w-5" />}
          maxWidthClassName="max-w-md"
          testId={`memory-engine-${dialog}-dialog`}
          closePolicy={
            saving === dialog ? { escape: false, backdrop: false, button: false } : undefined
          }
          onClose={() => setDialog(null)}>
          {panel(dialog)}
        </ModalShell>
      )}

      {/* Onboarding embeds this tab to pick a provider; upcoming engines are noise there. */}
      {!embedded && <MemoryComingSoon />}
    </div>
  );
}
