import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../../lib/i18n/I18nContext';
import type { Locale } from '../../../lib/i18n/types';
import localeReducer from '../../../store/localeSlice';
import CustomSearchPage from './CustomSearchPage';

const navigateMock = vi.fn();
const setDraftMock = vi.fn();

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

vi.mock('../../../components/settings/panels/SearchPanel', () => ({
  default: () => <div data-testid="search-panel">Search Panel</div>,
}));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({ snapshot: { sessionToken: 'header.payload.local' } }),
}));

vi.mock('../OnboardingContext', () => ({
  useOnboardingContext: () => ({
    draft: { connectedSources: [] },
    setDraft: setDraftMock,
    completeAndExit: vi.fn(),
  }),
}));

function renderPage() {
  const store = configureStore({
    reducer: { locale: localeReducer },
    preloadedState: { locale: { current: 'en' as Locale } },
  });

  return render(
    <Provider store={store}>
      <MemoryRouter>
        <I18nProvider>
          <CustomSearchPage />
        </I18nProvider>
      </MemoryRouter>
    </Provider>
  );
}

describe('CustomSearchPage', () => {
  beforeEach(() => {
    navigateMock.mockReset();
    setDraftMock.mockReset();
  });

  it('forces configure mode and hides the default/configure chooser for local sessions', () => {
    renderPage();

    expect(screen.getByTestId('search-panel')).toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-custom-search-step-default')).not.toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-custom-search-step-configure')).not.toBeInTheDocument();
  });
});
