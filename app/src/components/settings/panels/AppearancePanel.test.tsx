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

function renderPanel(
  fontSize: 'small' | 'medium' | 'large' | 'xlarge' = 'medium',
  customFontSizePx: number | null = null
) {
  return renderWithProviders(<AppearancePanel />, {
    preloadedState: {
      theme: {
        mode: 'system',
        tabBarLabels: 'hover',
        fontSize,
        customFontSizePx,
        agentMessageViewMode: 'bubbles',
      },
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

  it('reflects the effective size on the slider and highlights a matching preset', () => {
    // 18px == the Large preset, so the slider reads 18 and Large stays checked.
    const { getByTestId, getByRole } = renderPanel('medium', 18);
    expect((getByTestId('font-size-slider') as HTMLInputElement).value).toBe('18');
    const group = getByRole('radiogroup', { name: 'settings.appearance.fontSizeAria' });
    expect(within(group).getByRole('radio', { name: /fontSizeLarge/ })).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('dispatches a clamped custom px as the slider moves', () => {
    const { getByTestId, store } = renderPanel('medium');
    fireEvent.change(getByTestId('font-size-slider'), { target: { value: '26' } });
    expect(store.getState().theme.customFontSizePx).toBe(26);
  });

  it('commits the numeric field on blur, clamped to the supported range', () => {
    const { getByTestId, store } = renderPanel('medium');
    const field = within(getByTestId('font-size-custom-number')).getByRole('spinbutton');
    fireEvent.change(field, { target: { value: '99' } });
    fireEvent.blur(field);
    expect(store.getState().theme.customFontSizePx).toBe(28);
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
