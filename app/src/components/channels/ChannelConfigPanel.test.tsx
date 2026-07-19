import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { renderWithProviders } from '../../test/test-utils';
import ChannelConfigPanel from './ChannelConfigPanel';

// The credential form owns its own RPC/redux wiring (covered by its own suite);
// stub it so this test isolates the panel's channel→component routing.
vi.mock('./CredentialChannelConfig', () => ({
  default: ({ definition }: { definition: { id: string } }) => (
    <div data-testid="credential-config">{definition.id}</div>
  ),
}));

describe('<ChannelConfigPanel />', () => {
  it('routes the email channel to the credential form (#4280)', () => {
    renderWithProviders(
      <ChannelConfigPanel selectedChannel="email" definitions={FALLBACK_DEFINITIONS} />
    );
    const form = screen.getByTestId('credential-config');
    expect(form).toHaveTextContent('email');
  });

  it('routes lark and dingtalk to the same credential form', () => {
    for (const channel of ['lark', 'dingtalk'] as const) {
      const { unmount } = renderWithProviders(
        <ChannelConfigPanel selectedChannel={channel} definitions={FALLBACK_DEFINITIONS} />
      );
      expect(screen.getByTestId('credential-config')).toHaveTextContent(channel);
      unmount();
    }
  });
});
