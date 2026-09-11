import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { billingApi } from '../../../services/api/billingApi';
import type { PlanTier } from '../../../types/api';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import Button from '../../ui/Button';
import { SettingsStatusLine } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';
import SubscriptionPlans from './billing/SubscriptionPlans';
import { buildPlanId } from './billingHelpers';

const BillingPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const [currentTier, setCurrentTier] = useState<PlanTier>('FREE');
  const [billingInterval, setBillingInterval] = useState<'monthly' | 'annual'>('monthly');
  const [paymentMethod, setPaymentMethod] = useState<'card' | 'crypto'>('card');
  const [isPurchasing, setIsPurchasing] = useState(false);
  const [purchasingTier, setPurchasingTier] = useState<PlanTier | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [planLoading, setPlanLoading] = useState(true);
  const [planKnown, setPlanKnown] = useState(false);
  const paymentConfirmed = false;

  useEffect(() => {
    billingApi
      .getCurrentPlan()
      .then(data => {
        setCurrentTier(data.plan);
        setPlanKnown(true);
      })
      .catch(err => setError(err instanceof Error ? err.message : String(err)))
      .finally(() => setPlanLoading(false));
  }, []);

  const handleSetPaymentMethod = (method: 'card' | 'crypto') => {
    setPaymentMethod(method);
    if (method === 'crypto') setBillingInterval('annual');
  };

  const handleUpgrade = async (tier: PlanTier): Promise<void> => {
    setError(null);
    setIsPurchasing(true);
    setPurchasingTier(tier);
    try {
      if (paymentMethod === 'crypto') {
        const charge = await billingApi.createCoinbaseCharge(tier);
        await openUrl(charge.hostedUrl);
      } else {
        const session = await billingApi.purchasePlan(buildPlanId(tier, billingInterval));
        if (session.checkoutUrl) {
          await openUrl(session.checkoutUrl);
        } else {
          throw new Error('Checkout session did not return a redirect URL');
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsPurchasing(false);
      setPurchasingTier(null);
    }
  };

  return (
    <SettingsPanel>
      <SettingsStatusLine saving={false} error={error} savingLabel="" />
      <SubscriptionPlans
        currentTier={currentTier}
        billingInterval={billingInterval}
        setBillingInterval={setBillingInterval}
        paymentMethod={paymentMethod}
        setPaymentMethod={handleSetPaymentMethod}
        isPurchasing={isPurchasing}
        purchasingTier={purchasingTier}
        paymentConfirmed={paymentConfirmed}
        upgradesDisabled={planLoading || !planKnown}
        onUpgrade={handleUpgrade}
      />

      <div className="flex flex-wrap gap-3">
        <Button
          type="button"
          variant="secondary"
          size="md"
          onClick={() => void openUrl(BILLING_DASHBOARD_URL)}>
          {t('settings.billing.openDashboard')}
        </Button>
        <Button type="button" variant="tertiary" size="md" onClick={navigateBack}>
          {t('settings.billing.backToSettings')}
        </Button>
      </div>
    </SettingsPanel>
  );
};

export default BillingPanel;
