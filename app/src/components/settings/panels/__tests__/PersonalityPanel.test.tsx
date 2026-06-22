import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import PersonalityPanel from '../PersonalityPanel';

// The tab bodies have their own test suites — stub them so these tests stay
// focused on the hash <-> tab mapping that PersonalityPanel owns.
vi.mock('../PersonaPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-persona" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../MascotPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-mascot" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

describe('PersonalityPanel', () => {
  test('default hash renders the Personality tab with the embedded persona editor', () => {
    renderWithProviders(<PersonalityPanel />, { initialEntries: ['/settings/personality'] });

    expect(screen.getByTestId('personality-tab-personality')).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(screen.getByTestId('stub-persona')).toHaveAttribute('data-embedded', 'true');
    expect(screen.queryByTestId('stub-mascot')).not.toBeInTheDocument();
  });

  test('#face hash selects the Face tab with the embedded mascot panel', () => {
    renderWithProviders(<PersonalityPanel />, { initialEntries: ['/settings/personality#face'] });

    expect(screen.getByTestId('personality-tab-face')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('stub-mascot')).toHaveAttribute('data-embedded', 'true');
    expect(screen.queryByTestId('stub-persona')).not.toBeInTheDocument();
  });

  test('clicking the Face tab switches the view in place', async () => {
    renderWithProviders(<PersonalityPanel />, { initialEntries: ['/settings/personality'] });

    fireEvent.click(screen.getByTestId('personality-tab-face'));

    await screen.findByTestId('stub-mascot');
    expect(screen.queryByTestId('stub-persona')).not.toBeInTheDocument();
  });

  test('clicking Personality from the Face tab restores the persona editor', async () => {
    renderWithProviders(<PersonalityPanel />, { initialEntries: ['/settings/personality#face'] });

    fireEvent.click(screen.getByTestId('personality-tab-personality'));

    await screen.findByTestId('stub-persona');
    expect(screen.queryByTestId('stub-mascot')).not.toBeInTheDocument();
  });
});
