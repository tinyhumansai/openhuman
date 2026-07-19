import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import ChipTabs from '../../layout/ChipTabs';
import PageSectionHeader from '../../layout/PageSectionHeader';
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
    <div className="flex h-full min-h-0 flex-col gap-4" data-testid="wallet-panel">
      <PageSectionHeader
        title={t('pages.settings.account.walletBalances')}
        description={t('connections.header.wallet')}
        tabs={
          <ChipTabs<WalletTab>
            as="tab"
            ariaLabel={t('wallet.ariaLabel')}
            testIdPrefix="wallet"
            className="inline-flex flex-wrap items-center gap-1.5"
            items={[
              { id: 'balance', label: t('wallet.tabs.balance') },
              { id: 'recovery', label: t('wallet.tabs.recovery') },
            ]}
            value={tab}
            onChange={setTab}
          />
        }
      />
      <div className="min-h-0 flex-1 overflow-hidden rounded-2xl border border-line bg-surface shadow-subtle">
        {tab === 'balance' ? <WalletBalancesPanel /> : <RecoveryPhrasePanel />}
      </div>
    </div>
  );
}
