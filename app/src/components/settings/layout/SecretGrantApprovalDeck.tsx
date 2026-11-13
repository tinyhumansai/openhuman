/**
 * SecretGrantApprovalDeck — asks the user for the native secret master grant
 * while they are in Settings.
 *
 * Saving an API key (or any other config secret) from Settings seals it
 * through TinySecurity, which needs a one-time `native_secret_grant` from a
 * person. That save is a plain RPC with no chat thread behind it, so the core
 * parks the grant as an unrouted pending approval and the `approval_request`
 * event has no thread to land on. This deck reads the durable pending queue
 * (`useUnroutedApprovals`) and shows only that grant, above whichever settings
 * panel is saving. The save RPC stays blocked until the user decides: approve
 * and it completes, deny or let it expire and it fails with the reason.
 *
 * Other unrouted approvals (background triggers, #6406) stay on the chat
 * route's deck; only the grant the Settings save is waiting on is shown here.
 */
import debug from 'debug';

import { ApprovalCardAdapter } from '../../../features/conversations/aui/ApprovalCardAdapter';
import { useUnroutedApprovals } from '../../../hooks/useUnroutedApprovals';
import { useT } from '../../../lib/i18n/I18nContext';

const log = debug('settings:secret-grant');

/** The tool name the core's secret cipher asks the approval gate for. */
export const NATIVE_SECRET_GRANT_TOOL = 'native_secret_grant';

/**
 * The module's grant window is short, so the queue is re-read every 2 s while
 * Settings is open — the same cadence a watched flow run uses.
 */
const SECRET_GRANT_POLL_INTERVAL_MS = 2_000;

const SecretGrantApprovalDeck = () => {
  const { t } = useT();
  const { approvals, decidingId, error, decide } = useUnroutedApprovals(
    true,
    SECRET_GRANT_POLL_INTERVAL_MS
  );
  const grants = approvals.filter(approval => approval.tool_name === NATIVE_SECRET_GRANT_TOOL);
  if (grants.length === 0) return null;
  log('showing %d pending secret grant(s)', grants.length);

  return (
    <div className="flex flex-col gap-2 px-4 pt-4" data-testid="secret-grant-approval-deck">
      {error && (
        <p className="text-xs text-destructive" role="alert">
          {error}
        </p>
      )}
      {grants.map(grant => (
        <ApprovalCardAdapter
          key={grant.request_id}
          ariaLabel={t('settings.secretGrant.title')}
          title={t('settings.secretGrant.title')}
          subtitle={t('settings.secretGrant.subtitle')}
          command={grant.tool_name}
          toolName={grant.tool_name}
          expiresAt={grant.expires_at}
          analyticsPrefix="settings-secret-grant"
          testId="secret-grant-approval-card"
          busy={decidingId !== null}
          onDecide={decision => decide(grant.request_id, decision)}
        />
      ))}
    </div>
  );
};

export default SecretGrantApprovalDeck;
