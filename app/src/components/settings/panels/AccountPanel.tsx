import { ExternalLink } from 'lucide-react';

import { useUsageState } from '../../../hooks/useUsageState';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import type { PlanTier } from '../../../types/api';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import LanguageSelect from '../../LanguageSelect';
import TimezoneSelect from '../../TimezoneSelect';
import { Badge, Button, Card, Field, Progress, TileGrid } from '../../ui';
import SettingsPanel from '../layout/SettingsPanel';
import LogoutAndClearActions from '../LogoutAndClearActions';
import { PLANS } from './billingHelpers';

function planName(tier: PlanTier): string {
  return PLANS.find(plan => plan.tier === tier)?.name ?? tier;
}

/**
 * Settings → Account. One card per concern, top to bottom: who is signed in,
 * their plan and the billing dashboard, then the session and destructive
 * actions owned by {@link LogoutAndClearActions}. Sibling pages (Privacy,
 * Security, Import, …) are reached through the sidebar, not from here.
 */
const AccountPanel = () => {
  const { t } = useT();
  const { snapshot } = useCoreState();
  const user = snapshot.currentUser;
  // The usage hook fetches the live plan; the snapshot's embedded
  // subscription is the fallback until it lands (or when billing is offline).
  const { currentPlan, teamUsage, usagePct } = useUsageState();

  const name = user ? [user.firstName, user.lastName].filter(Boolean).join(' ') || null : null;
  const tier: PlanTier | null = currentPlan?.plan ?? user?.subscription?.plan ?? null;
  const usd = (n: number) => `$${n.toFixed(2)}`;
  const resetsOn = teamUsage?.cycleEndsAt ? new Date(teamUsage.cycleEndsAt) : null;

  return (
    <SettingsPanel
      testId="account-panel"
      description={t('pages.settings.accountSection.description')}>
      {/* ── Profile & plan: who is signed in, what they're on, how much is
          left this cycle, and the one place to manage billing. ─────────── */}
      {/* Profile and Preferences are short cards, so they share a row. */}
      <TileGrid columns={2}>
        {user && (
          <Card data-testid="account-profile" className="h-full">
            <div className="flex items-center justify-between gap-4 p-4">
              <div className="min-w-0">
                {name && (
                  <div className="truncate text-base font-semibold text-content">{name}</div>
                )}
                <div className="mt-0.5 text-xs text-content-muted">
                  {t('settings.account.signedIn')}
                </div>
              </div>
              {tier && (
                <Badge variant={tier === 'FREE' ? 'neutral' : 'primary'} data-testid="account-plan">
                  {planName(tier)}
                </Badge>
              )}
            </div>

            {teamUsage && teamUsage.cycleBudgetUsd > 0 && (
              <div className="space-y-2 px-4 py-3" data-testid="account-usage">
                <div className="flex items-baseline justify-between gap-3 text-xs">
                  <span className="font-medium text-content">
                    {t('settings.account.usageThisCycle')}
                  </span>
                  <span className="tabular-nums text-content-muted">
                    {t('settings.account.usageOf')
                      .replace('{spent}', usd(teamUsage.cycleSpentUsd))
                      .replace('{budget}', usd(teamUsage.cycleBudgetUsd))}
                  </span>
                </div>
                <Progress value={Math.round(usagePct * 100)} />
                {resetsOn && !Number.isNaN(resetsOn.getTime()) && (
                  <div className="text-[11px] text-content-faint">
                    {t('settings.account.resetsOn').replace(
                      '{date}',
                      resetsOn.toLocaleDateString()
                    )}
                  </div>
                )}
              </div>
            )}

            <Field
              label={t('settings.account.manageBilling')}
              description={t('settings.account.manageBillingDesc')}
              control={
                <Button
                  variant="secondary"
                  size="sm"
                  trailingIcon={<ExternalLink className="h-3.5 w-3.5" aria-hidden />}
                  onClick={() => void openUrl(`${BILLING_DASHBOARD_URL}?tab=billing`)}
                  data-testid="account-open-billing">
                  {t('settings.account.openDashboard')}
                </Button>
              }
            />
          </Card>
        )}

        {/* ── Preferences ─────────────────────────────────────────────────── */}
        <Card title={t('settings.account.preferences')} className="h-full">
          <Field
            label={t('settings.language')}
            description={t('settings.languageDesc')}
            control={<LanguageSelect ariaLabel={t('settings.language')} />}
          />
          <Field
            label={t('settings.timezone')}
            description={t('settings.timezoneDesc')}
            control={<TimezoneSelect ariaLabel={t('settings.timezone')} />}
          />
        </Card>
      </TileGrid>

      {/* ── Session: log out, or wipe this device ───────────────────────── */}
      <LogoutAndClearActions />
    </SettingsPanel>
  );
};

export default AccountPanel;
