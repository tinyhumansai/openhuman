import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../../lib/i18n/I18nContext';
import type { Locale } from '../../../lib/i18n/types';
import localeReducer from '../../../store/localeSlice';
import VaultSetupStep from './VaultSetupStep';

const navigateMock = vi.fn();
const setDraftMock = vi.fn();
const completeAndExitMock = vi.fn();

let sessionToken = 'header.payload.local';

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

vi.mock('../../../components/settings/panels/MemoryDataPanel', () => ({
  default: () => <div data-testid="memory-data-panel">Memory Data Panel</div>,
}));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({ snapshot: { sessionToken } }),
}));

vi.mock('../OnboardingContext', () => ({
  useOnboardingContext: () => ({
    draft: { connectedSources: [], customChoices: {} },
    setDraft: setDraftMock,
    completeAndExit: completeAndExitMock,
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
          <VaultSetupStep />
        </I18nProvider>
      </MemoryRouter>
    </Provider>
  );
}

describe('VaultSetupStep', () => {
  beforeEach(() => {
    navigateMock.mockReset();
    setDraftMock.mockReset();
    completeAndExitMock.mockReset();
    sessionToken = 'header.payload.local';
  });

  it('forces configure mode and hides chooser cards for local sessions', () => {
    renderPage();

    expect(screen.getByTestId('memory-data-panel')).toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-custom-vault-step-default')).not.toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-custom-vault-step-configure')).not.toBeInTheDocument();
  });

  it('shows default/configure chooser cards for managed sessions', () => {
    sessionToken = 'header.payload.remote';
    renderPage();

    expect(screen.getByTestId('onboarding-custom-vault-step-default')).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-custom-vault-step-configure')).toBeInTheDocument();
  });
});
