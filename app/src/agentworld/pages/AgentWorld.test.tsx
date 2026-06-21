import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, test, vi } from 'vitest';

import AgentWorld from './AgentWorld';

vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}));

vi.mock('../../components/layout/shell/SidebarSlot', () => ({
  SidebarContent: ({ children }: { children: ReactNode }) => (
    <div data-testid="sidebar-slot">{children}</div>
  ),
}));

vi.mock('../../components/layout/TwoPaneNav', () => ({
  default: ({
    selected,
    onSelect,
    groups,
  }: {
    selected: string;
    onSelect: (slug: string) => void;
    groups: Array<{ items: Array<{ value: string; label: string }> }>;
  }) => (
    <nav aria-label="agent-world-nav">
      <div data-testid="selected-section">{selected}</div>
      {groups.flatMap(group =>
        group.items.map(item => (
          <button key={item.value} type="button" onClick={() => onSelect(item.value)}>
            {item.label}
          </button>
        ))
      )}
    </nav>
  ),
}));

vi.mock('../components/WalletAddressChip', () => ({ default: () => <span>wallet-chip</span> }));
vi.mock('./WorldSection', () => ({ default: () => <div>world-section</div> }));
vi.mock('./FeedSection', () => ({ default: () => <div>feed-section</div> }));
vi.mock('./LedgerSection', () => ({ default: () => <div>ledger-section</div> }));
vi.mock('./JobsSection', () => ({ default: () => <div>jobs-section</div> }));
vi.mock('./BountiesSection', () => ({ default: () => <div>bounties-section</div> }));
vi.mock('./ExploreSection', () => ({ default: () => <div>explore-section</div> }));
vi.mock('./DirectorySection', () => ({ default: () => <div>directory-section</div> }));
vi.mock('./ProfilesSection', () => ({ default: () => <div>profiles-section</div> }));
vi.mock('./IdentitiesSection', () => ({ default: () => <div>identities-section</div> }));
vi.mock('./MarketplaceSection', () => ({ default: () => <div>marketplace-section</div> }));
vi.mock('./MessagingSection', () => ({ default: () => <div>messaging-section</div> }));

function renderAgentWorld(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/agent-world/*" element={<AgentWorld />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('AgentWorld', () => {
  test('defaults /agent-world to the TinyPlace world section', () => {
    renderAgentWorld('/agent-world');

    expect(screen.getByTestId('selected-section')).toHaveTextContent('world');
    expect(screen.getByText('world-section')).toBeInTheDocument();
  });

  test('uses framed section chrome outside the world route and navigates from the sidebar', async () => {
    const { container } = renderAgentWorld('/agent-world/feed');

    expect(screen.getByTestId('selected-section')).toHaveTextContent('feed');
    expect(screen.getByText('feed-section')).toBeInTheDocument();
    expect(container.querySelector('.max-w-6xl')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'agentWorld.directory' }));

    expect(await screen.findByText('directory-section')).toBeInTheDocument();
    expect(screen.getByTestId('selected-section')).toHaveTextContent('directory');
  });
});
