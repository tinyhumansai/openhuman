import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { renderWithProviders } from '../../test/test-utils';
import ChannelSelector from './ChannelSelector';

function renderSelector() {
  return renderWithProviders(
    <ChannelSelector
      definitions={FALLBACK_DEFINITIONS}
      selectedChannel="telegram"
      onSelectChannel={vi.fn()}
    />
  );
}

describe('<ChannelSelector /> channel logos (issue #2854)', () => {
  it('renders the Lark / Feishu brand logo on its card', () => {
    const { container } = renderSelector();
    expect(container.querySelector('img[src="/lark.png"]')).not.toBeNull();
  });

  it('renders the DingTalk brand logo on its card', () => {
    const { container } = renderSelector();
    expect(container.querySelector('img[src="/dingtalk.png"]')).not.toBeNull();
  });

  it('still renders an icon for every channel definition', () => {
    const { getByText } = renderSelector();
    // Every card label is present, and each card carries a visual icon node
    // (img logo, inline svg, or emoji span) — not just label text.
    for (const def of FALLBACK_DEFINITIONS) {
      const card = getByText(def.display_name).closest('button');
      expect(card).not.toBeNull();
      expect(card?.querySelector('img, svg, span.text-base')).not.toBeNull();
    }
  });
});
