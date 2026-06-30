import { fireEvent, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import AppearancePanel from './AppearancePanel';

// Pass-through translator so assertions can target the i18n keys directly.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ breadcrumbs: [], navigateBack: vi.fn() }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

function renderPanel(fontSize: 'small' | 'medium' | 'large' | 'xlarge' = 'medium') {
  return renderWithProviders(<AppearancePanel />, {
    preloadedState: {
      theme: { mode: 'system', tabBarLabels: 'hover', fontSize, agentMessageViewMode: 'bubbles' },
    },
  });
}

describe('<AppearancePanel /> font size', () => {
  it('renders the four font-size options as a radio group', () => {
    const { getByRole } = renderPanel();
    const group = getByRole('radiogroup', { name: 'settings.appearance.fontSizeAria' });
    const radios = within(group).getAllByRole('radio');
    expect(radios).toHaveLength(4);
  });

  it('marks the active font size as checked', () => {
    const { getByRole } = renderPanel('large');
    const group = getByRole('radiogroup', { name: 'settings.appearance.fontSizeAria' });
    const large = within(group).getByRole('radio', { name: /fontSizeLarge/ });
    expect(large).toHaveAttribute('aria-checked', 'true');
  });

  it('dispatches setFontSize when an option is clicked', () => {
    const { getByRole, store } = renderPanel('medium');
    const group = getByRole('radiogroup', { name: 'settings.appearance.fontSizeAria' });
    const xlarge = within(group).getByRole('radio', { name: /fontSizeXLarge/ });

    fireEvent.click(xlarge);

    expect(store.getState().theme.fontSize).toBe('xlarge');
  });

  it('toggles assistant text mode for chat output', () => {
    const { getByRole, store } = renderPanel('medium');
    const toggle = getByRole('switch', { name: /settings\.appearance\.assistantTextMode/ });

    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);

    expect(store.getState().theme.agentMessageViewMode).toBe('text');
  });

  it('toggles hide-agent-thinking on and off', () => {
    const { getByRole, store } = renderPanel('medium');
    const toggle = getByRole('switch', { name: /settings\.appearance\.hideAgentInsights/ });

    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);
    expect(store.getState().theme.hideAgentInsights).toBe(true);

    fireEvent.click(toggle);
    expect(store.getState().theme.hideAgentInsights).toBe(false);
  });
});
