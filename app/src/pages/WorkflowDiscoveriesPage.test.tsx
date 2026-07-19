import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import WorkflowDiscoveriesPage from './WorkflowDiscoveriesPage';

vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// PanelPage pulls i18n/redux we don't need — stub to bare markup that keeps the
// title + children so we can assert the page mounts the discoveries body.
vi.mock('../components/layout/PanelPage', () => ({
  default: ({ title, children }: { title: string; children: React.ReactNode }) => (
    <div>
      <h1>{title}</h1>
      {children}
    </div>
  ),
}));

// SuggestedWorkflows does its own RPCs — stub it; we only assert it's rendered.
vi.mock('../components/flows/SuggestedWorkflows', () => ({
  default: () => <div data-testid="suggested-workflows-stub" />,
}));

describe('WorkflowDiscoveriesPage', () => {
  it('renders the discoveries page title and mounts SuggestedWorkflows', () => {
    render(<WorkflowDiscoveriesPage />);
    expect(screen.getByText('flows.discoveries.title')).toBeInTheDocument();
    expect(screen.getByTestId('suggested-workflows-stub')).toBeInTheDocument();
  });
});
