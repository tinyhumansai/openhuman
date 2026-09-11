/**
 * GatewaySection — Settings → Core connection → "Run the core somewhere else".
 *
 * The core does not have to run inside this app. It can run in a Docker
 * container, on a machine reached over SSH, or in a container on a machine over
 * SSH — and this section is where a user configures and switches between those.
 *
 * Two things shape the UI here.
 *
 * **Reach and confinement are separate questions.** Where the core runs and
 * what contains it are independent, so the form asks them independently rather
 * than offering a flat list of combinations. "A container on the build server"
 * is not a third option to pick; it is the two answers chosen separately.
 *
 * **Activating takes real time and can fail in stages.** Creating a box,
 * pulling an image, booting the core and opening a tunnel are tens of seconds
 * with distinct failure modes, so the shell reports which step it is on and this
 * shows it — a bare spinner would say nothing about whether an image pull is
 * stuck.
 *
 * Lives beside `CoreConnectionPanel` rather than inside it because that file is
 * already near the repo's ~500-line guidance.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../../../../services/coreRpcClient';
import {
  activateGateway,
  activeGatewayId,
  deleteGateway,
  DESKTOP_GATEWAY_ID,
  type Gateway,
  gatewayStatus,
  type GatewayStatus,
  type GatewaySummary,
  listGateways,
  saveGateway,
} from '../../../../services/gatewayService';
import { setCoreMode } from '../../../../store/coreModeSlice';
import { useAppDispatch } from '../../../../store/hooks';
import { storeCoreMode, storeGatewayId } from '../../../../utils/configPersistence';
import Button from '../../../ui/Button';
import { SettingsRow, SettingsSection, SettingsTextField } from '../../controls';

const log = debug('settings:gateway');

/** The form's own shape, before it becomes a `GatewaySpec`. */
interface DraftGateway {
  id: string;
  label: string;
  /** Where it runs. */
  where: 'here' | 'ssh';
  /** What contains it. */
  contained: boolean;
  image: string;
  binary: string;
  destination: string;
  sshPort: string;
  identity: string;
  acceptNewHostKey: boolean;
}

function emptyDraft(): DraftGateway {
  return {
    id: '',
    label: '',
    where: 'here',
    contained: true,
    // What `docker-compose.yml` at the repo root builds and tags, so a
    // developer who has already run the compose stack has this image and the
    // form's default just works. `:latest` would be a tag nothing produces.
    image: 'openhuman-core:local',
    binary: '/usr/local/bin/openhuman-core',
    destination: '',
    sshPort: '',
    identity: '',
    acceptNewHostKey: false,
  };
}

/** Turn a draft into the record the shell stores, or explain why it cannot. */
export function draftToGateway(draft: DraftGateway): { gateway: Gateway } | { error: string } {
  const id = draft.id.trim();
  if (!id) return { error: 'idRequired' };
  if (id === DESKTOP_GATEWAY_ID) return { error: 'idReserved' };
  if (draft.where === 'ssh' && !draft.destination.trim()) return { error: 'destinationRequired' };
  if (draft.contained && !draft.image.trim()) return { error: 'imageRequired' };
  if (!draft.contained && !draft.binary.trim()) return { error: 'binaryRequired' };

  const port = draft.sshPort.trim();
  // A bare digit check would accept `0` and values past 65535, which then reach
  // the shell as connection settings and only fail later with a generic error.
  if (port && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65_535)) {
    return { error: 'portInvalid' };
  }

  return {
    gateway: {
      id,
      label: draft.label.trim() || id,
      spec: {
        kind: 'box',
        reach:
          draft.where === 'here'
            ? { kind: 'local' }
            : {
                kind: 'ssh',
                destination: draft.destination.trim(),
                ...(port ? { port: Number(port) } : {}),
                ...(draft.identity.trim() ? { identity: draft.identity.trim() } : {}),
                ...(draft.acceptNewHostKey ? { acceptNewHostKey: true } : {}),
              },
        confinement: draft.contained
          ? { kind: 'docker', image: draft.image.trim() }
          : { kind: 'passthrough', binary: draft.binary.trim() },
      },
    },
  };
}

interface Props {
  /** Whether this build can reach gateways at all. */
  available: boolean;
}

const GatewaySection = ({ available }: Props) => {
  const { t } = useT();
  const dispatch = useAppDispatch();

  const [gateways, setGateways] = useState<GatewaySummary[]>([]);
  const [activeId, setActiveId] = useState<string>(DESKTOP_GATEWAY_ID);
  const [status, setStatus] = useState<GatewayStatus>({ state: 'inactive' });
  const [draft, setDraft] = useState<DraftGateway | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    if (!available) return;
    const [listed, active] = await Promise.all([listGateways(), activeGatewayId()]);
    setGateways(listed);
    setActiveId(active);
    setStatus(await gatewayStatus(active));
  }, [available]);

  useEffect(() => {
    // Reading external state — the shell's gateway records and which one is
    // active — which is what an effect is for. The rule fires because `refresh`
    // ends in setState; it does so after an await, not synchronously, so there
    // is no cascading render. Same shape and same exemption as
    // `CoreConnectionPanel`'s live check.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh();
  }, [refresh]);

  // While a gateway is being provisioned the shell advances through named
  // steps. Poll only then: there is nothing to watch once it settles, and a
  // standing timer against an idle gateway is pure noise in the log.
  useEffect(() => {
    if (status.state !== 'activating') return undefined;
    const timer = setInterval(() => {
      void gatewayStatus(activeId).then(setStatus);
    }, 1_000);
    return () => clearInterval(timer);
  }, [status.state, activeId]);

  if (!available) return null;

  const handleActivate = async (id: string) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setActiveId(id);
    setStatus({ state: 'activating', step: '' });
    log('activating %s', id);
    try {
      await activateGateway(id);
      // The shell now answers `core_rpc_url` / `core_rpc_token` from this
      // gateway, so drop the renderer's cached copies — otherwise the next call
      // goes to the previous core with the previous bearer.
      clearCoreRpcUrlCache();
      clearCoreRpcTokenCache();
      if (id === DESKTOP_GATEWAY_ID) {
        storeCoreMode('local');
        dispatch(setCoreMode({ kind: 'local' }));
      } else {
        storeCoreMode('gateway');
        storeGatewayId(id);
        dispatch(setCoreMode({ kind: 'gateway', gatewayId: id }));
      }
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      log('activation failed: %s', reason);
      setError(reason);
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const handleSave = async () => {
    if (!draft || busy) return;
    const built = draftToGateway(draft);
    if ('error' in built) {
      setError(t(`settings.gateway.${built.error}`));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await saveGateway(built.gateway);
      setDraft(null);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (id: string) => {
    setBusy(true);
    setError(null);
    try {
      await deleteGateway(id);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const statusLine = (gateway: GatewaySummary): string | undefined => {
    if (gateway.id !== activeId) return undefined;
    switch (status.state) {
      case 'activating':
        return status.step
          ? t('settings.gateway.activatingStep').replace('{step}', status.step)
          : t('settings.gateway.activating');
      case 'connected':
        return t('settings.gateway.connected').replace('{endpoint}', status.endpoint);
      case 'failed':
        return t('settings.gateway.failed').replace('{reason}', status.reason);
      case 'inactive':
        return undefined;
    }
  };

  return (
    <SettingsSection title={t('settings.gateway.title')}>
      <div className="px-4 py-3">
        <p className="text-[11px] text-content-muted leading-relaxed">
          {t('settings.gateway.description')}
        </p>
      </div>

      {gateways.map(gateway => (
        <SettingsRow
          key={gateway.id}
          label={gateway.label}
          description={statusLine(gateway) ?? t(`settings.gateway.kind.${gateway.kind}`)}
          control={
            <div className="flex items-center gap-2">
              {gateway.id === activeId && (
                <span
                  className={`inline-block h-2.5 w-2.5 rounded-full flex-shrink-0 ${
                    status.state === 'connected'
                      ? 'bg-sage-500'
                      : status.state === 'activating'
                        ? 'bg-amber-400 animate-pulse'
                        : 'bg-coral-500'
                  }`}
                  aria-hidden="true"
                  data-testid={`gateway-dot-${gateway.id}`}
                />
              )}
              <Button
                variant="secondary"
                size="xs"
                analyticsId="gateway-activate"
                disabled={busy || gateway.id === activeId}
                onClick={() => void handleActivate(gateway.id)}
                data-testid={`gateway-use-${gateway.id}`}>
                {gateway.id === activeId ? t('settings.gateway.inUse') : t('settings.gateway.use')}
              </Button>
              {gateway.id !== DESKTOP_GATEWAY_ID && (
                <Button
                  variant="secondary"
                  size="xs"
                  analyticsId="gateway-remove"
                  disabled={busy}
                  onClick={() => void handleDelete(gateway.id)}
                  data-testid={`gateway-remove-${gateway.id}`}>
                  {t('settings.gateway.remove')}
                </Button>
              )}
            </div>
          }
        />
      ))}

      {draft ? (
        <div className="flex flex-col gap-3 px-4 py-4" data-testid="gateway-form">
          <div className="flex flex-col gap-1">
            <label htmlFor="gateway-id" className="text-xs font-medium text-content-secondary">
              {t('settings.gateway.nameLabel')}
            </label>
            <SettingsTextField
              id="gateway-id"
              value={draft.label}
              placeholder={t('settings.gateway.namePlaceholder')}
              onChange={e =>
                setDraft({
                  ...draft,
                  label: e.target.value,
                  // Derive the id from the name until the user has one, so the
                  // form asks one question instead of two.
                  id:
                    draft.id ||
                    e.target.value
                      .trim()
                      .toLowerCase()
                      .replace(/[^a-z0-9-]+/g, '-'),
                })
              }
            />
          </div>

          <fieldset className="flex flex-col gap-2">
            <legend className="text-xs font-medium text-content-secondary">
              {t('settings.gateway.whereLegend')}
            </legend>
            {(['here', 'ssh'] as const).map(where => (
              <label key={where} className="flex items-center gap-2 text-xs">
                <input
                  type="radio"
                  name="gateway-where"
                  checked={draft.where === where}
                  onChange={() => setDraft({ ...draft, where })}
                  data-analytics-id={`gateway-where-${where}`}
                />
                {t(`settings.gateway.where.${where}`)}
              </label>
            ))}
          </fieldset>

          {draft.where === 'ssh' && (
            <>
              <div className="flex flex-col gap-1">
                <label
                  htmlFor="gateway-destination"
                  className="text-xs font-medium text-content-secondary">
                  {t('settings.gateway.destinationLabel')}
                </label>
                <SettingsTextField
                  id="gateway-destination"
                  value={draft.destination}
                  placeholder={t('settings.gateway.destinationPlaceholder')}
                  onChange={e => setDraft({ ...draft, destination: e.target.value })}
                />
                <p className="text-[11px] text-content-muted leading-snug">
                  {t('settings.gateway.destinationHelp')}
                </p>
              </div>
              <div className="flex flex-col gap-1">
                <label
                  htmlFor="gateway-identity"
                  className="text-xs font-medium text-content-secondary">
                  {t('settings.gateway.identityLabel')}
                </label>
                <SettingsTextField
                  id="gateway-identity"
                  value={draft.identity}
                  placeholder={t('settings.gateway.identityPlaceholder')}
                  onChange={e => setDraft({ ...draft, identity: e.target.value })}
                />
              </div>
              <label className="flex items-center gap-2 text-xs">
                <input
                  type="checkbox"
                  checked={draft.acceptNewHostKey}
                  onChange={e => setDraft({ ...draft, acceptNewHostKey: e.target.checked })}
                  data-analytics-id="gateway-accept-new-host-key"
                />
                {t('settings.gateway.acceptNewHostKey')}
              </label>
              <p className="text-[11px] text-content-muted leading-snug">
                {t('settings.gateway.acceptNewHostKeyHelp')}
              </p>
            </>
          )}

          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={draft.contained}
              onChange={e => setDraft({ ...draft, contained: e.target.checked })}
              data-analytics-id="gateway-contained"
            />
            {t('settings.gateway.containedLabel')}
          </label>

          <div className="flex flex-col gap-1">
            <label htmlFor="gateway-target" className="text-xs font-medium text-content-secondary">
              {draft.contained
                ? t('settings.gateway.imageLabel')
                : t('settings.gateway.binaryLabel')}
            </label>
            <SettingsTextField
              id="gateway-target"
              mono
              value={draft.contained ? draft.image : draft.binary}
              onChange={e =>
                setDraft(
                  draft.contained
                    ? { ...draft, image: e.target.value }
                    : { ...draft, binary: e.target.value }
                )
              }
            />
          </div>

          <div className="flex items-center gap-3">
            <Button
              size="sm"
              analyticsId="gateway-save"
              disabled={busy}
              onClick={() => void handleSave()}
              data-testid="gateway-save">
              {t('settings.gateway.save')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              analyticsId="gateway-cancel"
              onClick={() => {
                setDraft(null);
                setError(null);
              }}>
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      ) : (
        <div className="px-4 py-3">
          <Button
            variant="secondary"
            size="sm"
            analyticsId="gateway-add"
            onClick={() => setDraft(emptyDraft())}
            data-testid="gateway-add">
            {t('settings.gateway.add')}
          </Button>
        </div>
      )}

      {error && (
        <p className="px-4 pb-3 text-xs text-coral-600" data-testid="gateway-error">
          {error}
        </p>
      )}
    </SettingsSection>
  );
};

export default GatewaySection;
