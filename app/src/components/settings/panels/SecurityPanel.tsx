import { KeyRound, ShieldCheck } from 'lucide-react';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  fetchToolTrust,
  revokeToolTrust,
  type ToolTrustGrant,
} from '../../../services/api/approvalApi';
import { decideKeyringConsent, retryKeyringProbe } from '../../../services/keyringApi';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
import { TileGrid } from '../../ui/TileGrid';
import { SettingsBadge, SettingsSection, SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const MODE_BADGE_VARIANT: Record<string, 'success' | 'warning' | 'neutral' | 'danger'> = {
  os_keyring: 'success',
  // Operator-configured encrypted_file backend: secrets.enc, unlocked by a
  // master key in the OS keychain. A deliberate posture, not a degraded one.
  local_encrypted_file: 'success',
  // Consent fallback after the OS keyring failed.
  local_encrypted: 'warning',
  // Plaintext dev-keychain.json, no encryption at all.
  local_plaintext_file: 'danger',
  consent_pending: 'neutral',
  declined: 'danger',
};

/**
 * `activeMode` arrives snake_case from the core; the `keyring.settings.mode.*`
 * i18n keys are camelCase. Without this map every lookup missed and the badge
 * rendered the literal key ("keyring.settings.mode.os_keyring") — `declined`
 * was the only mode that ever resolved, because it is spelled the same in both
 * conventions.
 *
 * An unrecognised mode deliberately falls through to the raw value, keeping the
 * existing version-skew behaviour: the badge shows what the core reported
 * instead of silently mislabelling it as a mode this build does know.
 */
const MODE_I18N_KEY: Record<string, string> = {
  os_keyring: 'osKeychain',
  local_encrypted: 'encryptedFile',
  local_encrypted_file: 'localEncryptedFile',
  local_plaintext_file: 'localPlaintextFile',
  consent_pending: 'consentPending',
  declined: 'declined',
};

const SecurityPanel = () => {
  const { snapshot } = useCoreState();
  const { t } = useT();
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toolTrust, setToolTrust] = useState<ToolTrustGrant[]>([]);
  const [toolTrustLoading, setToolTrustLoading] = useState(true);
  const [toolTrustError, setToolTrustError] = useState<string | null>(null);
  const [revokingRequestId, setRevokingRequestId] = useState<string | null>(null);

  const keyringStatus = snapshot.keyringStatus;
  const modeBadgeVariant =
    MODE_BADGE_VARIANT[keyringStatus.activeMode] ?? MODE_BADGE_VARIANT.consent_pending;
  const modeI18nKey = MODE_I18N_KEY[keyringStatus.activeMode] ?? keyringStatus.activeMode;

  const loadToolTrust = async () => {
    setToolTrustLoading(true);
    setToolTrustError(null);
    try {
      setToolTrust(await fetchToolTrust());
    } catch {
      setToolTrustError(t('keyring.settings.toolTrustLoadFailed'));
    } finally {
      setToolTrustLoading(false);
    }
  };

  useEffect(() => {
    void Promise.resolve().then(() => loadToolTrust());
    // Load on mount. Retry is explicit after an error.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleRevokeToolTrust = async (requestId: string) => {
    setRevokingRequestId(requestId);
    setToolTrustError(null);
    try {
      await revokeToolTrust(requestId);
      setToolTrust(current => current.filter(grant => grant.request_id !== requestId));
    } catch {
      setToolTrustError(t('keyring.settings.toolTrustRevokeFailed'));
    } finally {
      setRevokingRequestId(null);
    }
  };

  const handleRetryProbe = async () => {
    setIsLoading(true);
    setError(null);
    try {
      await retryKeyringProbe();
    } catch {
      setError(t('keyring.settings.retryFailed'));
    } finally {
      setIsLoading(false);
    }
  };

  const handleConsentChange = async (mode: 'local_encrypted' | 'declined') => {
    setIsLoading(true);
    setError(null);
    try {
      await decideKeyringConsent(mode);
    } catch {
      setError(t('keyring.consent.error'));
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <SettingsPanel description={t('pages.settings.account.securityDesc')}>
      <>
        {/* Storage mode and keychain availability are two short status
            cards, so they sit side by side rather than stacking full-width. */}
        <TileGrid columns={2}>
          <Card title={t('keyring.settings.storageMode')} className="h-full" divided={false}>
            <div className="flex items-center gap-3 p-4">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-surface-muted text-content-secondary">
                <KeyRound className="h-4.5 w-4.5" aria-hidden />
              </span>
              <div className="min-w-0 flex-1">
                <SettingsBadge variant={modeBadgeVariant}>
                  {t(`keyring.settings.mode.${modeI18nKey}` as Parameters<typeof t>[0])}
                </SettingsBadge>
                <p className="mt-1 text-xs text-content-muted">
                  {t('keyring.settings.backend')}: {keyringStatus.backendName}
                </p>
              </div>
            </div>
          </Card>

          <Card title={t('keyring.settings.availability')} className="h-full" divided={false}>
            <div className="flex items-center gap-3 p-4">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-surface-muted text-content-secondary">
                <ShieldCheck className="h-4.5 w-4.5" aria-hidden />
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span
                    className={`h-2 w-2 shrink-0 rounded-full ${keyringStatus.available ? 'bg-sage-500' : 'bg-amber-500'}`}
                  />
                  <span className="text-sm text-content-secondary">
                    {keyringStatus.available
                      ? t('keyring.settings.available')
                      : t('keyring.settings.unavailable')}
                  </span>
                </div>
                {keyringStatus.failureReason && (
                  <p className="mt-1 text-xs text-content-muted">{keyringStatus.failureReason}</p>
                )}
              </div>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => void handleRetryProbe()}
                disabled={isLoading}>
                {isLoading ? t('keyring.consent.retrying') : t('keyring.settings.retryButton')}
              </Button>
            </div>
          </Card>
        </TileGrid>

        {/* Consent management (only when keyring is unavailable) */}
        {!keyringStatus.available && (
          <SettingsSection title={t('keyring.settings.consentTitle')}>
            <div className="px-4 py-3 space-y-3">
              <p className="text-xs text-content-muted">
                {t('keyring.settings.consentDescription')}
              </p>
              <div className="flex flex-wrap gap-2">
                {keyringStatus.activeMode !== 'local_encrypted' && (
                  <Button
                    type="button"
                    variant="primary"
                    size="sm"
                    onClick={() => void handleConsentChange('local_encrypted')}
                    disabled={isLoading}>
                    {t('keyring.settings.grantConsent')}
                  </Button>
                )}
                {keyringStatus.activeMode !== 'declined' && (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => void handleConsentChange('declined')}
                    disabled={isLoading}>
                    {t('keyring.settings.revokeConsent')}
                  </Button>
                )}
              </div>
            </div>
          </SettingsSection>
        )}

        <SettingsSection title={t('keyring.settings.toolTrustTitle')}>
          <div className="space-y-3 px-4 py-3">
            <p className="text-xs text-content-muted">
              {t('keyring.settings.toolTrustDescription')}
            </p>
            {toolTrustLoading ? (
              <p className="text-sm text-content-muted" role="status">
                {t('keyring.settings.toolTrustLoading')}
              </p>
            ) : toolTrustError && toolTrust.length === 0 ? (
              <div className="flex flex-wrap items-center gap-3" role="alert">
                <p className="text-sm text-coral-600 dark:text-coral-200">{toolTrustError}</p>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => void loadToolTrust()}>
                  {t('keyring.settings.toolTrustRetry')}
                </Button>
              </div>
            ) : toolTrust.length === 0 ? (
              <p className="text-sm text-content-muted">{t('keyring.settings.toolTrustEmpty')}</p>
            ) : (
              <ul className="divide-y divide-line">
                {toolTrust.map(grant => (
                  <li
                    key={grant.request_id}
                    className="flex items-center justify-between gap-3 py-3">
                    <span className="min-w-0 truncate text-sm text-content">{grant.tool}</span>
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      onClick={() => void handleRevokeToolTrust(grant.request_id)}
                      disabled={revokingRequestId !== null}>
                      {revokingRequestId === grant.request_id
                        ? t('keyring.settings.toolTrustRevoking')
                        : t('keyring.settings.toolTrustRevoke')}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
            {toolTrustError && toolTrust.length > 0 && (
              <p className="text-sm text-coral-600 dark:text-coral-200" role="alert">
                {toolTrustError}
              </p>
            )}
          </div>
        </SettingsSection>

        <SettingsStatusLine
          saving={isLoading}
          error={error}
          savingLabel={t('keyring.consent.retrying')}
        />
      </>
    </SettingsPanel>
  );
};

export default SecurityPanel;
