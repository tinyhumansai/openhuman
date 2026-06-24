import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it } from 'vitest';

import userErrorsReducer, { reportUserError } from '../../../store/userErrorsSlice';
import type { UserErrorDescriptor } from '../../../types/userError';
import UserErrorCenter from '../UserErrorCenter';

const budget: UserErrorDescriptor = {
  id: 'budget_exceeded:chat:unknown',
  kind: 'budget_exceeded',
  severity: 'warning',
  scope: 'chat',
  titleKey: 'userErrors.budgetExceeded.title',
  bodyKey: 'userErrors.budgetExceeded.body',
  action: 'open_billing',
};
const credits: UserErrorDescriptor = {
  id: 'insufficient_credits:chat:unknown',
  kind: 'insufficient_credits',
  severity: 'warning',
  scope: 'chat',
  titleKey: 'userErrors.insufficientCredits.title',
  bodyKey: 'userErrors.insufficientCredits.body',
  action: 'open_provider_settings',
};

/** Probe that renders the current pathname so tests can assert navigation. */
function LocationProbe() {
  const { pathname } = useLocation();
  return <div data-testid="pathname">{pathname}</div>;
}

function renderCenter(descriptors: UserErrorDescriptor[] = []) {
  const store = configureStore({ reducer: { userErrors: userErrorsReducer } });
  descriptors.forEach((d, i) => store.dispatch(reportUserError({ descriptor: d, at: 1000 + i })));
  const utils = render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/chat']}>
        <UserErrorCenter />
        <LocationProbe />
      </MemoryRouter>
    </Provider>
  );
  return { store, ...utils };
}

describe('UserErrorCenter', () => {
  beforeEach(() => {
    /* fresh store per render */
  });

  it('renders nothing when there are no active errors', () => {
    renderCenter([]);
    expect(screen.queryByTestId('user-error-center')).toBeNull();
  });

  it('shows a badge with the active count and opens the panel on click', () => {
    renderCenter([budget, credits]);
    expect(screen.getByTestId('user-error-badge')).toHaveTextContent('2');
    expect(screen.queryByTestId('user-error-panel')).toBeNull();

    fireEvent.click(screen.getByTestId('user-error-trigger'));
    expect(screen.getByTestId('user-error-panel')).toBeInTheDocument();
    expect(screen.getAllByTestId('user-error-item')).toHaveLength(2);
    // Localized copy resolves through the default (English) i18n map.
    expect(screen.getByText('Managed budget reached')).toBeInTheDocument();
    expect(screen.getByText('Provider credits required')).toBeInTheDocument();
  });

  it('routes the budget action to billing and resolves the entry', () => {
    const { store } = renderCenter([budget]);
    fireEvent.click(screen.getByTestId('user-error-trigger'));
    fireEvent.click(screen.getByTestId('user-error-action'));
    expect(screen.getByTestId('pathname')).toHaveTextContent('/settings/billing');
    // Resolved → drops out of the active list, so the center unmounts.
    expect(store.getState().userErrors.byId[budget.id].resolvedAt).toBeDefined();
    expect(screen.queryByTestId('user-error-center')).toBeNull();
  });

  it('routes the credits action to provider settings', () => {
    renderCenter([credits]);
    fireEvent.click(screen.getByTestId('user-error-trigger'));
    fireEvent.click(screen.getByTestId('user-error-action'));
    expect(screen.getByTestId('pathname')).toHaveTextContent('/settings/llm');
  });

  it('dismiss removes the entry without navigating', () => {
    const { store } = renderCenter([budget]);
    fireEvent.click(screen.getByTestId('user-error-trigger'));
    fireEvent.click(screen.getByTestId('user-error-dismiss'));
    expect(screen.getByTestId('pathname')).toHaveTextContent('/chat');
    expect(store.getState().userErrors.order).not.toContain(budget.id);
    expect(screen.queryByTestId('user-error-center')).toBeNull();
  });
});
