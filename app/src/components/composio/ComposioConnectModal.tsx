/**
 * Modal for connecting / managing a Composio toolkit.
 *
 * Mirrors the flow, positioning, and portal/backdrop plumbing of
 * `SkillSetupModal` so the two feel identical to the user:
 *
 *   disconnected → collect provider-specific required fields (if any) →
 *   "Connect" button → POST composio_authorize → open connectUrl via
 *   tauri-opener → poll listConnections until the toolkit flips to
 *   ACTIVE → "Connected" success screen with a "Disconnect" action.
 *
 * Provider-specific required fields (Jira subdomain, WhatsApp WABA id,
 * Dynamics 365 org name, …) are declared in the
 * [`toolkitRequiredFields`] registry rather than hard-coded as per-toolkit
 * booleans here (#2127). If Composio still returns
 * `ConnectedAccount_MissingRequiredFields` (error code 612) for any toolkit
 * — e.g. a new required field landed backend-side before the registry was
 * updated — the modal transitions to a `needs-fields` recovery phase that
 * collects the same registry fields and retries, instead of surfacing the
 * raw backend error.
 *
 * Redundant refetches from the polling hook in `useComposioIntegrations`
 * keep the Skills page badge in sync too, so the card reflects the new
 * state as soon as the modal closes.
 *
 * The connect/poll/disconnect/scope state machine lives in
 * `useComposioConnectFlow`, and the auth-error helpers live in
 * `composioAuthErrors` — both split out purely to keep this file under the
 * repo's ~500-line budget. Re-exported below for existing consumers/tests.
 */
import type { ComposioConnection } from '../../lib/composio/types';
import { openUrl } from '../../utils/openUrl';
import { Button, Checkbox, ModalShell } from '../ui';
import { deriveConnectionLabel } from './composioAuthErrors';
import { RequiredFieldsForm } from './RequiredFieldsForm';
import { ScopeToggles } from './ScopeToggles';
import type { ComposioToolkitMeta } from './toolkitMeta';
import TriggerToggles from './TriggerToggles';
import { useComposioConnectFlow } from './useComposioConnectFlow';

export {
  isMissingRequiredFieldsError,
  isValidAtlassianSubdomain,
  sanitizeAuthError,
} from './composioAuthErrors';

interface ComposioConnectModalProps {
  toolkit: ComposioToolkitMeta;
  /** All existing connections for this toolkit (if any) from the hook. */
  connections?: ComposioConnection[];
  /** Connected, but not yet exposed to the agent tool surface. */
  agentUnsupported?: boolean;
  /** Invoked on successful connect/disconnect so the parent can refresh. */
  onChanged?: () => void;
  onClose: () => void;
}

export default function ComposioConnectModal({
  toolkit,
  connections,
  agentUnsupported = false,
  onChanged,
  onClose,
}: ComposioConnectModalProps) {
  const {
    t,
    phase,
    setPhase,
    error,
    setError,
    connectUrl,
    clearMemoryOnDisconnect,
    setClearMemoryOnDisconnect,
    requiredFields,
    fieldValues,
    setFieldValues,
    fieldErrors,
    setFieldErrors,
    activeConnections,
    activeConnection,
    scopes,
    scopeError,
    savingScope,
    connectInFlight,
    initiallyConnected,
    initiallyExpired,
    handleConnect,
    handleToggleScope,
    handleDisconnect,
  } = useComposioConnectFlow({ toolkit, connections, onChanged });

  const headerTitle =
    phase === 'connected'
      ? `${t('composio.connect.manage')} ${toolkit.name}`
      : phase === 'expired'
        ? `${t('composio.reconnect')} ${toolkit.name}`
        : `${t('composio.connect.connect')} ${toolkit.name}`;

  return (
    <ModalShell
      onClose={onClose}
      title={headerTitle}
      titleId="composio-setup-title"
      subtitle={toolkit.description}
      icon={toolkit.icon}
      maxWidthClassName="max-w-[460px]"
      contentClassName="px-5 py-4 space-y-3">
      {phase === 'idle' && (
        <>
          <p className="text-sm text-content-secondary">
            {`${t('composio.connect.idleDescription')} ${toolkit.name} ${t('composio.connect.idleDescriptionSuffix')}`}
          </p>
          <div className="rounded-xl border border-line bg-surface-muted p-3">
            <p className="mt-1 text-xs leading-relaxed text-content-secondary">
              {toolkit.name} {t('composio.connect.permissionsNote')}{' '}
              <span className="font-medium">{toolkit.permissionLabel}</span>.{' '}
              {t('composio.connect.permissionsNoteSuffix')}
            </p>
          </div>
          <RequiredFieldsForm
            fields={requiredFields}
            values={fieldValues}
            errors={fieldErrors}
            onChange={(key, v) => {
              setFieldValues(prev => ({ ...prev, [key]: v }));
              if (fieldErrors[key]) {
                setFieldErrors(prev => {
                  const next = { ...prev };
                  delete next[key];
                  return next;
                });
              }
            }}
          />
          {error && phase === 'idle' && <p className="text-[11px] text-coral-600">{error}</p>}
          <Button
            variant="primary"
            size="lg"
            disabled={connectInFlight}
            onClick={() => void handleConnect()}
            className="w-full">
            {`${t('composio.connect.connect')} ${toolkit.name}`}
          </Button>
        </>
      )}

      {phase === 'needs-fields' && (
        <>
          <p className="text-sm text-content-secondary">
            {`${t('composio.connect.needsFieldsPrefix')} ${toolkit.name} ${t('composio.connect.needsFieldsSuffix')}`}
          </p>
          <RequiredFieldsForm
            fields={requiredFields}
            values={fieldValues}
            errors={fieldErrors}
            autoFocusFirst
            onChange={(key, v) => {
              setFieldValues(prev => ({ ...prev, [key]: v }));
              if (fieldErrors[key]) {
                setFieldErrors(prev => {
                  const next = { ...prev };
                  delete next[key];
                  return next;
                });
              }
            }}
          />
          <Button
            variant="primary"
            size="lg"
            disabled={connectInFlight}
            onClick={() => void handleConnect()}
            className="w-full">
            {t('composio.connect.retryConnection')}
          </Button>
          <Button
            variant="secondary"
            size="md"
            onClick={() => {
              setPhase('idle');
              setFieldErrors({});
              setError(null);
            }}
            className="w-full">
            {t('common.cancel')}
          </Button>
        </>
      )}

      {phase === 'authorizing' && (
        <p className="text-sm text-content-muted">{t('composio.connect.requestingUrl')}</p>
      )}

      {phase === 'waiting' && (
        <>
          <div className="flex items-center gap-2 text-sm text-content-secondary">
            <div className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
            {`${t('composio.connect.waitingFor')} ${toolkit.name} ${t('composio.connect.oauthComplete')}`}
          </div>
          {connectUrl && (
            <Button
              variant="secondary"
              size="md"
              onClick={() => void openUrl(connectUrl)}
              className="w-full">
              {t('composio.connect.reopenBrowser')}
            </Button>
          )}
          <p className="text-xs text-content-faint">{t('composio.connect.waitingHint')}</p>
        </>
      )}

      {phase === 'expired' && (
        <>
          <div className="rounded-xl border border-coral-200 bg-coral-50 p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-coral-800">
              <div className="w-2 h-2 rounded-full bg-coral-500" />
              {t('composio.expiredAuthorization').replace('{name}', toolkit.name)}
            </div>
            <p className="mt-2 text-xs leading-relaxed text-coral-700">
              {t('composio.expiredDescription').replace('{name}', toolkit.name)}
            </p>
          </div>
          <Button
            variant="primary"
            size="lg"
            disabled={connectInFlight}
            onClick={() => void handleConnect()}
            className="w-full">
            {`${t('composio.reconnect')} ${toolkit.name}`}
          </Button>
        </>
      )}

      {phase === 'connected' && (
        <>
          {/* Single connection: inline status (backward-compatible view) */}
          {activeConnections.length <= 1 && (
            <div className="flex items-center gap-2 text-sm text-sage-700">
              <div className="w-2 h-2 rounded-full bg-sage-500" />
              <div>
                {`${toolkit.name} ${t('composio.connect.isConnected')}`} &nbsp;
                {(activeConnections[0] ?? activeConnection) &&
                  deriveConnectionLabel(activeConnections[0] ?? activeConnection!) && (
                    <span className="text-[11px] text-content-faint font-mono">
                      ({deriveConnectionLabel((activeConnections[0] ?? activeConnection)!)})
                    </span>
                  )}
              </div>
            </div>
          )}
          {/* Multiple connections: list with per-connection controls */}
          {activeConnections.length > 1 && (
            <div className="space-y-2">
              <p className="text-xs font-medium text-content-muted uppercase tracking-wide">
                {t('composio.connect.connectedAccounts')} ({activeConnections.length})
              </p>
              {activeConnections.map(conn => (
                <div
                  key={conn.id}
                  className="flex items-center justify-between gap-2 rounded-lg border border-line bg-surface-muted px-3 py-2">
                  <div className="flex items-center gap-2 min-w-0">
                    <div className="w-2 h-2 rounded-full bg-sage-500 shrink-0" />
                    <span className="text-sm text-content truncate">
                      {deriveConnectionLabel(conn) ?? toolkit.name}
                    </span>
                    {conn.id === activeConnections[0]?.id && (
                      <span className="text-[10px] font-medium text-primary-600 dark:text-primary-400 bg-primary-50 dark:bg-primary-500/10 px-1.5 py-0.5 rounded-full shrink-0">
                        {t('composio.connect.defaultLabel')}
                      </span>
                    )}
                  </div>
                  <Button
                    variant="tertiary"
                    tone="danger"
                    size="xs"
                    onClick={() => void handleDisconnect(conn)}
                    className="shrink-0">
                    {t('composio.connect.disconnectAccount')}
                  </Button>
                </div>
              ))}
            </div>
          )}
          {agentUnsupported && (
            <div className="rounded-xl border border-amber-200 bg-amber-50 p-3 dark:border-amber-500/30 dark:bg-amber-500/10">
              <div className="flex items-center gap-2 text-sm font-medium text-amber-800 dark:text-amber-200">
                <div className="h-2 w-2 rounded-full bg-amber-500" />
                {t('composio.previewBadge')}
              </div>
              <p className="mt-2 text-xs leading-relaxed text-amber-700 dark:text-amber-200/80">
                {t('composio.previewTooltip')}
              </p>
            </div>
          )}
          <ScopeToggles
            scopes={scopes}
            savingScope={savingScope}
            onToggle={handleToggleScope}
            error={scopeError}
          />
          {activeConnection && (
            <TriggerToggles
              toolkitSlug={toolkit.slug}
              toolkitName={toolkit.name}
              connectionId={activeConnection.id}
            />
          )}
          <Button
            variant="secondary"
            size="lg"
            disabled={connectInFlight}
            onClick={() => void handleConnect()}
            className="w-full">
            {t('composio.connect.addAnotherAccount')}
          </Button>
          <label
            htmlFor="composio-clear-memory-on-disconnect"
            className="flex items-start gap-2 rounded-lg border border-line bg-surface-muted px-3 py-2">
            <Checkbox
              id="composio-clear-memory-on-disconnect"
              checked={clearMemoryOnDisconnect}
              onCheckedChange={setClearMemoryOnDisconnect}
              className="mt-0.5"
            />
            <span className="min-w-0">
              <span className="block text-sm font-medium text-content">
                {t('accounts.disconnectClearMemory')}
              </span>
              <span className="block text-xs text-content-muted">
                {t('accounts.disconnectClearMemoryHint')}
              </span>
            </span>
          </label>
          <div className="grid grid-cols-2 gap-3">
            <Button
              variant="secondary"
              tone="danger"
              size="lg"
              onClick={() => void handleDisconnect()}
              className="w-full">
              {t('skills.disconnect')}
            </Button>
            <Button variant="primary" size="lg" onClick={onClose} className="w-full">
              {t('common.close')}
            </Button>
          </div>
        </>
      )}

      {phase === 'disconnecting' && (
        <p className="text-sm text-content-muted">{t('composio.connect.disconnecting')}</p>
      )}

      {phase === 'error' && (
        <>
          <div className="rounded-xl border border-coral-200 bg-coral-50 p-3">
            <p className="text-sm text-coral-700">{error ?? t('misc.somethingWentWrong')}</p>
          </div>
          <Button
            variant="secondary"
            size="md"
            onClick={() => {
              setClearMemoryOnDisconnect(false);
              setPhase(initiallyConnected ? 'connected' : initiallyExpired ? 'expired' : 'idle');
              setError(null);
            }}
            className="w-full">
            {t('common.dismiss')}
          </Button>
        </>
      )}
    </ModalShell>
  );
}
