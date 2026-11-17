import { fireEvent, render, within } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { setChatMascotDismissed } from '../../../store/mascotSlice';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import AppearancePanel from './AppearancePanel';

// Pass-through translator so assertions can target the i18n keys directly.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ breadcrumbs: [], navigateBack: vi.fn() }),
}));

function renderPanel(
  fontSize: 'small' | 'medium' | 'large' | 'xlarge' = 'medium',
  customFontSizePx: number | null = null,
  initialEntries?: string[]
) {
  return renderWithProviders(<AppearancePanel />, {
    preloadedState: {
      theme: { mode: 'system', tabBarLabels: 'hover', fontSize, customFontSizePx },
    },
    initialEntries,
  });
}

// Theme Studio (the theme gallery + customizer) now lives on its own route
// (`/settings/theme`, `ThemeStudioPanel`) rather than as a tab inside
// AppearancePanel — see ThemeStudioPanel.test.tsx for that coverage.
describe('<AppearancePanel />', () => {
  it('lets a dismissed chat mascot be shown again', () => {
    const store = createTestStore();
    store.dispatch(setChatMascotDismissed(true));
    const { getByRole } = renderWithProviders(<AppearancePanel />, { store });
    const toggle = getByRole('switch', { name: 'settings.appearance.showChatMascot' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);
    expect(store.getState().mascot.chatMascotDismissed).toBe(false);
  });
  it('renders the font size card and no theme gallery/customizer of its own', () => {
    const { getByTestId, getByRole, queryByTestId } = renderPanel();
    expect(getByTestId('font-size-card')).toBeInTheDocument();
    expect(
      getByRole('radiogroup', { name: 'settings.appearance.fontSizeAria' })
    ).toBeInTheDocument();
    expect(queryByTestId('theme-gallery')).not.toBeInTheDocument();
    expect(queryByTestId('theme-customize')).not.toBeInTheDocument();
  });

  it('redirects to /settings/theme for the legacy #studio hash', () => {
    // A real `<Routes>` switches AppearancePanel out for whatever matches
    // /settings/theme, so verify the redirect the way the app actually
    // renders it rather than inspecting the pre-navigation render.
    const store = createTestStore({
      theme: { mode: 'system', tabBarLabels: 'hover', fontSize: 'medium', customFontSizePx: null },
    });
    const { getByTestId, queryByTestId } = render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/settings/appearance#studio']}>
          <Routes>
            <Route path="/settings/appearance" element={<AppearancePanel />} />
            <Route path="/settings/theme" element={<div data-testid="theme-route" />} />
          </Routes>
        </MemoryRouter>
      </Provider>
    );
    expect(getByTestId('theme-route')).toBeInTheDocument();
    expect(queryByTestId('font-size-card')).not.toBeInTheDocument();
  });
});

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
    // The slider migrated from a raw `<input type="range">` to the Radix-backed
    // `Slider` primitive (ui/Slider.tsx): the thumb carries `role="slider"` +
    // `aria-valuenow` instead of an `<input>` `.value`.
    const { getByRole } = renderPanel('medium', 18);
    expect(getByRole('slider')).toHaveAttribute('aria-valuenow', '18');
    const group = getByRole('radiogroup', { name: 'settings.appearance.fontSizeAria' });
    expect(within(group).getByRole('radio', { name: /fontSizeLarge/ })).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('dispatches a clamped custom px as the slider moves', () => {
    // Radix `Slider` has no layout in jsdom (pointer drags all resolve to the
    // same zero-width rect), so keyboard is the reliable interaction path here
    // too — same rationale as ui/Slider.test.tsx's own arrow-key coverage.
    const { getByRole, store } = renderPanel('medium');
    const thumb = getByRole('slider');
    thumb.focus();
    fireEvent.keyDown(thumb, { key: 'ArrowRight' });
    expect(store.getState().theme.customFontSizePx).toBe(17);
  });

  it('commits the numeric field on blur, clamped to the supported range', () => {
    const { getByTestId, store } = renderPanel('medium');
    const field = within(getByTestId('font-size-custom-number')).getByRole('spinbutton');
    fireEvent.change(field, { target: { value: '99' } });
    fireEvent.blur(field);
    expect(store.getState().theme.customFontSizePx).toBe(28);
  });
});
