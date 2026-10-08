import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import de from '../../lib/i18n/de';
import { I18nProvider } from '../../lib/i18n/I18nContext';
import pt from '../../lib/i18n/pt';
import localeReducer, { setLocale } from '../../store/localeSlice';
import { WHAT_LEAVES_ITEMS } from './whatLeavesItems';
import WhatLeavesLink from './WhatLeavesLink';
import WhatLeavesMyComputerSheet from './WhatLeavesMyComputerSheet';

describe('WhatLeavesMyComputerSheet', () => {
  it('renders nothing when closed', () => {
    render(<WhatLeavesMyComputerSheet open={false} onClose={() => {}} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('lists all configured honest leave items when open', () => {
    render(<WhatLeavesMyComputerSheet open={true} onClose={() => {}} />);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    for (const item of WHAT_LEAVES_ITEMS) {
      expect(screen.getByText(item.title)).toBeInTheDocument();
    }
    expect(WHAT_LEAVES_ITEMS).toHaveLength(4);
    expect(screen.getByText(/timing and token usage data to Langfuse/)).toBeInTheDocument();
    expect(
      screen.getByText(/Sharing and content capture both start turned off/)
    ).toBeInTheDocument();
  });

  it('calls onClose when Got it is clicked', () => {
    const onClose = vi.fn();
    render(<WhatLeavesMyComputerSheet open={true} onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: 'Got it' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('translates the analytics and trace consent disclosures for the active locale', () => {
    const store = configureStore({ reducer: { locale: localeReducer } });
    store.dispatch(setLocale('de'));
    render(
      <Provider store={store}>
        <I18nProvider>
          <WhatLeavesMyComputerSheet open={true} onClose={() => {}} />
        </I18nProvider>
      </Provider>
    );
    for (const key of [
      'privacy.whatLeaves.analytics.title',
      'privacy.whatLeaves.analytics.body',
      'privacy.whatLeaves.traces.title',
      'privacy.whatLeaves.traces.body',
    ]) {
      expect(screen.getByText(de[key])).toBeInTheDocument();
    }
  });

  it('calls onClose on Escape', () => {
    const onClose = vi.fn();
    render(<WhatLeavesMyComputerSheet open={true} onClose={onClose} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('names OpenHuman as the backend owner in the Portuguese disclosure', () => {
    const store = configureStore({ reducer: { locale: localeReducer } });
    store.dispatch(setLocale('pt'));
    render(
      <Provider store={store}>
        <I18nProvider>
          <WhatLeavesMyComputerSheet open={true} onClose={() => {}} />
        </I18nProvider>
      </Provider>
    );
    expect(screen.getByText(pt['privacy.whatLeaves.traces.title'])).toBeInTheDocument();
    expect(screen.getByText(/Langfuse por meio do backend do OpenHuman/)).toBeInTheDocument();
  });
});

describe('WhatLeavesLink', () => {
  it('opens the sheet when clicked', () => {
    render(<WhatLeavesLink />);
    expect(screen.queryByRole('dialog')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'What leaves my computer?' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('accepts a custom label', () => {
    render(<WhatLeavesLink label="Network activity?" />);
    expect(screen.getByRole('button', { name: 'Network activity?' })).toBeInTheDocument();
  });
});
