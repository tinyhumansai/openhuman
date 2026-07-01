import { configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../../lib/i18n/I18nContext';
import type { Locale } from '../../../lib/i18n/types';
import localeReducer from '../../../store/localeSlice';
import type { ReadinessReport } from '../../../utils/tauriCommands/readiness';
import ReadinessPanel from './ReadinessPanel';

const checkAllMock = vi.fn();

vi.mock('../../../utils/tauriCommands/readiness', () => ({
  readinessCheckAll: () => checkAllMock(),
}));

function report(overrides: Partial<ReadinessReport> = {}): ReadinessReport {
  return {
    host_os: 'macos',
    overall: 'ok',
    model_connection_ok: true,
    checks: [
      {
        id: 'core_health',
        category: 'core',
        status: 'ok',
        title: 'Core service',
        detail: '3/3 core components healthy.',
        platform: 'macos',
        required: true,
      },
      {
        id: 'model_connection',
        category: 'model',
        status: 'fail',
        title: 'Model connection',
        detail: 'Could not validate a model connection.',
        remediation: 'Check your API key, then retry.',
        platform: 'macos',
        required: true,
      },
    ],
    ...overrides,
  };
}

function renderPanel(props: Parameters<typeof ReadinessPanel>[0] = {}) {
  const store = configureStore({
    reducer: { locale: localeReducer },
    preloadedState: { locale: { current: 'en' as Locale } },
  });
  return render(
    <Provider store={store}>
      <I18nProvider>
        <ReadinessPanel {...props} />
      </I18nProvider>
    </Provider>
  );
}

describe('ReadinessPanel', () => {
  beforeEach(() => {
    checkAllMock.mockReset();
  });

  it('renders each check from the report with its status', async () => {
    checkAllMock.mockResolvedValue(report());
    renderPanel();

    await waitFor(() =>
      expect(screen.getByTestId('readiness-check-core_health')).toBeInTheDocument()
    );
    expect(screen.getByTestId('readiness-check-core_health')).toHaveAttribute('data-status', 'ok');
    expect(screen.getByTestId('readiness-check-model_connection')).toHaveAttribute(
      'data-status',
      'fail'
    );
  });

  it('shows remediation text and a retry button only for non-ok checks', async () => {
    checkAllMock.mockResolvedValue(report());
    renderPanel();

    await waitFor(() =>
      expect(screen.getByTestId('readiness-check-model_connection')).toBeInTheDocument()
    );
    expect(screen.getByText('Check your API key, then retry.')).toBeInTheDocument();
    // fail row has retry; ok row does not
    expect(screen.getByTestId('readiness-retry-model_connection')).toBeInTheDocument();
    expect(screen.queryByTestId('readiness-retry-core_health')).not.toBeInTheDocument();
  });

  it('notifies parent of the model-connection gate status', async () => {
    checkAllMock.mockResolvedValue(report({ model_connection_ok: false }));
    const onModelStatusChange = vi.fn();
    renderPanel({ onModelStatusChange });

    await waitFor(() => expect(onModelStatusChange).toHaveBeenCalledWith(false));
  });

  it('re-runs checks when "run again" is clicked', async () => {
    checkAllMock.mockResolvedValue(report());
    renderPanel();
    await waitFor(() => expect(checkAllMock).toHaveBeenCalledTimes(1));

    await userEvent.click(screen.getByTestId('readiness-run-all'));
    await waitFor(() => expect(checkAllMock).toHaveBeenCalledTimes(2));
  });

  it('surfaces a load error and reports the gate as not-ok', async () => {
    checkAllMock.mockRejectedValue(new Error('core offline'));
    const onModelStatusChange = vi.fn();
    renderPanel({ onModelStatusChange });

    await waitFor(() => expect(screen.getByTestId('readiness-load-error')).toBeInTheDocument());
    expect(onModelStatusChange).toHaveBeenCalledWith(false);
  });
});
