import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import ChannelConnectHelp from './ChannelConnectHelp';

describe('<ChannelConnectHelp /> (issue #4884)', () => {
  it('renders grounded connect steps for Discord', () => {
    const { getByText } = renderWithProviders(<ChannelConnectHelp channelId="discord" />);
    expect(getByText('How to connect')).toBeInTheDocument();
    expect(getByText(/Discord developer portal/i)).toBeInTheDocument();
  });

  it('renders grounded connect steps for Telegram', () => {
    const { getByText } = renderWithProviders(<ChannelConnectHelp channelId="telegram" />);
    expect(getByText('How to connect')).toBeInTheDocument();
    expect(getByText(/@BotFather/i)).toBeInTheDocument();
  });

  it('renders nothing for a channel without documented guidance', () => {
    const { container } = renderWithProviders(<ChannelConnectHelp channelId="lark" />);
    expect(container).toBeEmptyDOMElement();
  });
});
