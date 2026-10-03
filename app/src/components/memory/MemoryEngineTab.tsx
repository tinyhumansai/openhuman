/**
 * Memory → Engine: pick who stores and answers, in the provider-list shape the
 * web-search settings use.
 *
 * - TinyHumans (hosted) needs a signed-in account; signed out, its row is
 *   disabled and says why.
 * - CortexDB (self-hosted) opens a connect dialog for its endpoint and key.
 *
 * The status banner reads `memory_engine_get`: off (with the reason, or the
 * generic "sign in or connect your own" explanation), degraded or down.
 *
 * debug logging: DEBUG=openhuman:memory:engine
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import {
  type EngineDescriptor,
  type EngineSetRequest,
  type EngineState,
  isMemoryOn,
  memoryEngineSet,
  memoryEnginesList,
  memoryErrorMessage,
} from '../../services/api/memoryApi';
import { isLocalSessionToken } from '../../utils/localSession';
import { ProviderGroup, ProviderListRow } from '../settings/panels/ai/ProviderListRow';
import { Alert, AlertDescription, AlertTitle, Badge, Button } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryEngineConnectDialog from './MemoryEngineConnectDialog';

const log = debug('openhuman:memory:engine');

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

  const [engines, setEngines] = useState<EngineDescriptor[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dialogFor, setDialogFor] = useState<EngineDescriptor | null>(null);

  useEffect(() => {
    let cancelled = false;
    memoryEnginesList()
      .then(list => {
        if (cancelled) return;
        log('engines: %d active=%s', list.engines.length, list.active ?? 'none');
        setEngines(list.engines);
      })
      .catch(err => {
        if (cancelled) return;
        log('engines list failed: %o', err);
        setLoadError(memoryErrorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const select = useCallback(
    async (req: EngineSetRequest): Promise<boolean> => {
      setSaving(true);
      setSaveError(null);
      try {
        const next = await memoryEngineSet(req);
        log('engine set: %s status=%s', next.engine ?? 'none', next.status);
        onStateChange(next);
        return true;
      } catch (err) {
        log('engine set failed: %o', err);
        setSaveError(memoryErrorMessage(err));
        return false;
      } finally {
        setSaving(false);
      }
    },
    [onStateChange]
  );

  const activeId = state?.engine ?? null;
  const on = isMemoryOn(state);

  const statusBanner = (() => {
    if (!state) return null;
    if (!on) {
      return (
        <Alert variant="info" data-testid="memory-engine-status-off">
          <AlertTitle>{t('memoryPage.off.title')}</AlertTitle>
          <AlertDescription>
            {state.reason || t('memoryPage.engine.offExplanation')}
          </AlertDescription>
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

  const rowDetail = (engine: EngineDescriptor): string => {
    if (engine.hosted && !signedIn) return t('memoryPage.engine.signInRequired');
    if (engine.id === activeId && state?.endpoint) return state.endpoint;
    if (engine.hosted) return t('memoryPage.engine.hostedDetail');
    return engine.default_endpoint || t('memoryPage.engine.selfHostedDetail');
  };

  const choose = (engine: EngineDescriptor) => {
    if (engine.needs_endpoint || engine.needs_key) {
      setDialogFor(engine);
      return;
    }
    void select({ engine: engine.id });
  };

  return (
    <div
      className={embedded ? 'space-y-4' : 'space-y-4 animate-fade-up'}
      data-testid="memory-engine-tab">
      {statusBanner}
      {saveError !== null && (
        <Alert variant="destructive" data-testid="memory-engine-save-error">
          <AlertDescription>{saveError}</AlertDescription>
        </Alert>
      )}

      {loadError !== null ? (
        <Alert variant="destructive" data-testid="memory-engine-load-error">
          <AlertTitle>{t('memoryPage.engine.loadError')}</AlertTitle>
          <AlertDescription>{loadError}</AlertDescription>
        </Alert>
      ) : engines === null ? (
        <CenteredLoadingState label={t('memoryPage.loading')} />
      ) : (
        <ProviderGroup
          title={t('memoryPage.engine.listTitle')}
          description={t('memoryPage.engine.listDescription')}
          card
          data-testid="memory-engines">
          {engines.map(engine => {
            const isActive = engine.id === activeId;
            const blocked = engine.hosted && !signedIn;
            const testId = `memory-engine-${engine.id}`;
            const configurable = engine.needs_endpoint || engine.needs_key;
            return (
              <ProviderListRow
                key={engine.id}
                slug={engine.id}
                label={engine.label}
                tone=""
                detail={<span data-testid={`${testId}-detail`}>{rowDetail(engine)}</span>}
                detailMono={!blocked && !engine.hosted}
                badge={
                  isActive ? (
                    <Badge variant={on ? 'success' : 'warning'} data-testid={`${testId}-active`}>
                      {on ? t('memoryPage.engine.active') : t('memoryPage.engine.statusOff')}
                    </Badge>
                  ) : null
                }
                control={
                  isActive && !configurable ? null : (
                    <Button
                      type="button"
                      size="sm"
                      variant={isActive ? 'secondary' : 'primary'}
                      disabled={saving || blocked}
                      data-testid={`${testId}-use`}
                      onClick={() => choose(engine)}>
                      {isActive ? t('memoryPage.engine.edit') : t('memoryPage.engine.use')}
                    </Button>
                  )
                }
                data-testid={testId}
              />
            );
          })}
        </ProviderGroup>
      )}

      {dialogFor && (
        <MemoryEngineConnectDialog
          engine={dialogFor}
          currentEndpoint={dialogFor.id === activeId ? state?.endpoint : undefined}
          keySaved={dialogFor.id === activeId && Boolean(state?.has_key)}
          saving={saving}
          onSubmit={select}
          onClose={() => setDialogFor(null)}
        />
      )}
    </div>
  );
}
