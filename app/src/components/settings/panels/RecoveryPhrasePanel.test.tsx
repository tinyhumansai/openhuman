import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import RecoveryPhrasePanel from './RecoveryPhrasePanel';

const navigateBackMock = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: navigateBackMock, breadcrumbs: [] }),
}));

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: { currentUser: { id: 'test-user', publicKey: 'test-pubkey' } },
    setEncryptionKey: vi.fn(),
  }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title, description }: { title: string; description?: string }) => (
    <div data-testid="settings-header">
      {title} - {description}
    </div>
  ),
}));

vi.mock('../../../features/wallet/setupLocalWalletFromMnemonic', () => ({
  persistLocalWalletFromMnemonic: vi.fn(),
}));

describe('<RecoveryPhrasePanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('initially hides the recovery phrase and reveals it when clicking the reveal button', () => {
    render(<RecoveryPhrasePanel />);

    const copyButton = screen.getByText('mnemonic.copyToClipboard').closest('button')!;
    expect(copyButton).toBeDisabled();

    const revealButton = screen.getByLabelText('mnemonic.revealPhrase');
    expect(revealButton).toBeInTheDocument();

    fireEvent.click(revealButton);

    expect(screen.queryByLabelText('mnemonic.revealPhrase')).not.toBeInTheDocument();
    expect(copyButton).not.toBeDisabled();
  });
});
