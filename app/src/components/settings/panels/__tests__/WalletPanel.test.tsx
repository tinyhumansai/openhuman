import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import WalletPanel from '../WalletPanel';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../WalletBalancesPanel', () => ({ default: () => <div>balance-panel</div> }));
vi.mock('../RecoveryPhrasePanel', () => ({ default: () => <div>recovery-panel</div> }));

describe('WalletPanel', () => {
  it('defaults to the balance tab and switches to recovery', () => {
    render(<WalletPanel />);

    // Balance shown first.
    expect(screen.getByText('balance-panel')).toBeInTheDocument();
    expect(screen.queryByText('recovery-panel')).not.toBeInTheDocument();

    // Switch to the Recovery tab.
    fireEvent.click(screen.getByTestId('wallet-recovery'));
    expect(screen.getByText('recovery-panel')).toBeInTheDocument();
    expect(screen.queryByText('balance-panel')).not.toBeInTheDocument();
  });
});
