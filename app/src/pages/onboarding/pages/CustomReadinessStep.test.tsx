import { configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../../lib/i18n/I18nContext';
import type { Locale } from '../../../lib/i18n/types';
import localeReducer from '../../../store/localeSlice';
import type { ReadinessReport } from '../../../utils/tauriCommands/readiness';
import CustomReadinessStep from './CustomReadinessStep';

const navigateMock = vi.fn();
const completeAndExitMock = vi.fn().mockResolvedValue(undefined);
const checkAllMock = vi.fn();

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

vi.mock('../OnboardingContext', () => ({
  useOnboardingContext: () => ({
    draft: { connectedSources: [], customChoices: {} },
    setDraft: vi.fn(),
    completeAndExit: completeAndExitMock,
  }),
}));

vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

vi.mock('../../../utils/tauriCommands/readiness', () => ({
  readinessCheckAll: () => checkAllMock(),
}));

function report(modelOk: boolean): ReadinessReport {
  return {
    host_os: 'macos',
    overall: modelOk ? 'ok' : 'fail',
    model_connection_ok: modelOk,
    checks: [
      {
        id: 'model_connection',
        category: 'model',
        status: modelOk ? 'ok' : 'fail',
        title: 'Model connection',
        detail: modelOk ? 'Validated.' : 'Not validated.',
        platform: 'macos',
        required: true,
      },
    ],
  };
}

function renderStep() {
  const store = configureStore({
    reducer: { locale: localeReducer },
    preloadedState: { locale: { current: 'en' as Locale } },
  });
  return render(
    <Provider store={store}>
      <MemoryRouter>
        <I18nProvider>
          <CustomReadinessStep />
        </I18nProvider>
      </MemoryRouter>
    </Provider>
  );
}

describe('CustomReadinessStep', () => {
  beforeEach(() => {
    navigateMock.mockReset();
    completeAndExitMock.mockClear();
    checkAllMock.mockReset();
  });

  it('disables Finish until a model connection validates', async () => {
    checkAllMock.mockResolvedValue(report(false));
    renderStep();

    await waitFor(() => expect(screen.getByTestId('readiness-model-gate')).toBeInTheDocument());
    expect(screen.getByTestId('onboarding-next-button')).toBeDisabled();
  });

  it('enables Finish once the model connection is validated', async () => {
    checkAllMock.mockResolvedValue(report(true));
    renderStep();

    await waitFor(() => expect(screen.getByTestId('onboarding-next-button')).toBeEnabled());
    // gate hint is hidden once the model is ok
    expect(screen.queryByTestId('readiness-model-gate')).not.toBeInTheDocument();
  });

  it('lets the user skip the gate with the labeled toggle', async () => {
    checkAllMock.mockResolvedValue(report(false));
    renderStep();

    await waitFor(() => expect(screen.getByTestId('readiness-skip-toggle')).toBeInTheDocument());
    expect(screen.getByTestId('onboarding-next-button')).toBeDisabled();

    await userEvent.click(screen.getByTestId('readiness-skip-toggle'));
    expect(screen.getByTestId('onboarding-next-button')).toBeEnabled();
  });

  it('completes onboarding when Finish is clicked', async () => {
    checkAllMock.mockResolvedValue(report(true));
    renderStep();

    await waitFor(() => expect(screen.getByTestId('onboarding-next-button')).toBeEnabled());
    await userEvent.click(screen.getByTestId('onboarding-next-button'));
    await waitFor(() => expect(completeAndExitMock).toHaveBeenCalledTimes(1));
  });

  it('surfaces a finish error when completeAndExit fails', async () => {
    checkAllMock.mockResolvedValue(report(true));
    completeAndExitMock.mockRejectedValueOnce(new Error('network down'));
    renderStep();

    await waitFor(() => expect(screen.getByTestId('onboarding-next-button')).toBeEnabled());
    await userEvent.click(screen.getByTestId('onboarding-next-button'));
    await waitFor(() =>
      expect(screen.getByTestId('onboarding-readiness-exit-error')).toBeInTheDocument()
    );
  });
});
