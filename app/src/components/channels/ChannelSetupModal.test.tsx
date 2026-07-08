import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { renderWithProviders } from '../../test/test-utils';
import ChannelSetupModal from './ChannelSetupModal';

const larkDefinition = FALLBACK_DEFINITIONS.find(def => def.id === 'lark')!;
const emailDefinition = FALLBACK_DEFINITIONS.find(def => def.id === 'email')!;

describe('<ChannelSetupModal /> header logo (issue #2854)', () => {
  it('renders the Lark / Feishu brand logo in the modal header', () => {
    renderWithProviders(<ChannelSetupModal definition={larkDefinition} onClose={vi.fn()} />);
    expect(document.querySelector('img[src="/lark.png"]')).not.toBeNull();
  });
});

describe('<ChannelSetupModal /> credential-channel routing (#4280)', () => {
  it('renders the credential form for email instead of "config not available"', () => {
    const { getByPlaceholderText, queryByText } = renderWithProviders(
      <ChannelSetupModal definition={emailDefinition} onClose={vi.fn()} />
    );
    // The reused credential form renders the IMAP host field…
    expect(getByPlaceholderText('imap.fastmail.com')).toBeInTheDocument();
    // …and does not fall through to the not-available placeholder.
    expect(queryByText(/config not available/i)).toBeNull();
  });
});
