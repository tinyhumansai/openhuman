import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { renderWithProviders } from '../../test/test-utils';
import ChannelSetupModal from './ChannelSetupModal';

const larkDefinition = FALLBACK_DEFINITIONS.find(def => def.id === 'lark')!;

describe('<ChannelSetupModal /> header logo (issue #2854)', () => {
  it('renders the Lark / Feishu brand logo in the modal header', () => {
    renderWithProviders(<ChannelSetupModal definition={larkDefinition} onClose={vi.fn()} />);
    expect(document.querySelector('img[src="/lark.png"]')).not.toBeNull();
  });
});
