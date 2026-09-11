import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsTabbedPage from '../layout/SettingsTabbedPage';
import RecoveryPhrasePanel from './RecoveryPhrasePanel';
import WalletBalancesPanel from './WalletBalancesPanel';

type WalletTab = 'balance' | 'recovery';

/**
 * WalletPanel — the Connections "Wallet" destination as a two-tab view:
 * **Wallet balance** (multi-chain balances) and **Recovery** (recovery phrase).
 * A chip row switches between the two existing panels, which each keep their own
 * header + scroll.
 */
export default function WalletPanel() {
  const { t } = useT();
  const [tab, setTab] = useState<WalletTab>('balance');

  return (
    <SettingsTabbedPage
      title={t('pages.settings.account.walletBalances')}
      description={t('connections.header.wallet')}
      tabs={[
        { id: 'balance', label: t('wallet.tabs.balance') },
        { id: 'recovery', label: t('wallet.tabs.recovery') },
      ]}
      value={tab}
      onChange={setTab}
      tabsAriaLabel={t('wallet.ariaLabel')}
      tabsTestIdPrefix="wallet">
      <div className="min-h-0 h-full" data-testid="wallet-panel">
        {tab === 'balance' ? <WalletBalancesPanel /> : <RecoveryPhrasePanel />}
      </div>
    </SettingsTabbedPage>
  );
}
