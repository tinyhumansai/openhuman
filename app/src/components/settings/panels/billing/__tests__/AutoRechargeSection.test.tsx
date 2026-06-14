/**
 * AutoRechargeSection — unit tests for the auto-recharge UI component.
 *
 * Target lines: 93, 295, 372, 391
 *
 * 93  — SettingsSwitch rendered when not arLoading (the disabled= prop expression)
 * 295 — empty-state "no cards" branch
 * 372 — set-default button rendered for non-default card
 * 391 — confirm/cancel delete flow
 *
 * NOTE: useT is NOT mocked here — the default I18nContext (en.ts) is active,
 * so all text assertions use the actual English strings from en.ts.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AutoRechargeSettings, SavedCard } from '../../../../../services/api/creditsApi';
import AutoRechargeSection from '../AutoRechargeSection';

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeArSettings(overrides: Partial<AutoRechargeSettings> = {}): AutoRechargeSettings {
  return {
    enabled: true,
    thresholdUsd: 10,
    rechargeAmountUsd: 20,
    weeklyLimitUsd: 100,
    inFlight: false,
    spentThisWeekUsd: 0,
    weekStartDate: '2026-01-01',
    hasSavedPaymentMethod: true,
    lastTriggeredAt: null,
    lastRechargeAt: null,
    lastPaymentIntentId: null,
    lastError: null,
    ...overrides,
  };
}

function makeCard(overrides: Partial<SavedCard> = {}): SavedCard {
  return {
    id: 'card-1',
    brand: 'visa',
    last4: '4242',
    expMonth: 12,
    expYear: 2028,
    isDefault: false,
    billingDetails: {},
    ...overrides,
  };
}

const defaultProps = {
  arLoading: false,
  arError: null,
  arSaving: false,
  arThreshold: 10,
  arAmount: 20,
  arWeeklyLimit: 100,
  arDirty: false,
  setArThreshold: vi.fn(),
  setArAmount: vi.fn(),
  setArWeeklyLimit: vi.fn(),
  onArToggle: vi.fn(),
  onArSave: vi.fn(),
  cards: [] as SavedCard[],
  cardsLoading: false,
  confirmDeleteId: null,
  deletingCardId: null,
  settingDefaultId: null,
  setConfirmDeleteId: vi.fn(),
  onSetDefault: vi.fn(),
  onDeleteCard: vi.fn(),
  onAddCard: vi.fn(),
};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('AutoRechargeSection — toggle switch (line 93)', () => {
  it('renders the toggle switch when not loading (line 93)', () => {
    render(
      <AutoRechargeSection {...defaultProps} arSettings={makeArSettings({ enabled: false })} />
    );
    // en.ts: 'settings.billing.autoRecharge.toggleAriaLabel': 'Toggle auto-recharge'
    const toggle = screen.getByRole('switch', { name: /Toggle auto-recharge/i });
    expect(toggle).toBeInTheDocument();
  });

  it('shows loading skeleton instead of switch when arLoading=true', () => {
    render(<AutoRechargeSection {...defaultProps} arSettings={null} arLoading={true} />);
    // No switch rendered while loading (skeleton div shown instead)
    expect(screen.queryByRole('switch')).not.toBeInTheDocument();
  });

  it('calls onArToggle when switch is clicked', () => {
    const onArToggle = vi.fn();
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings({ enabled: false })}
        onArToggle={onArToggle}
      />
    );
    fireEvent.click(screen.getByRole('switch'));
    expect(onArToggle).toHaveBeenCalledTimes(1);
  });
});

describe('AutoRechargeSection — error banner', () => {
  it('renders error banner when arError is set', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        arError="Recharge failed: payment declined"
      />
    );
    expect(screen.getByText('Recharge failed: payment declined')).toBeInTheDocument();
  });
});

describe('AutoRechargeSection — enabled settings panel', () => {
  it('shows the threshold/amount/weeklyLimit selectors when enabled', () => {
    render(
      <AutoRechargeSection {...defaultProps} arSettings={makeArSettings({ enabled: true })} />
    );
    // en.ts: 'settings.billing.autoRecharge.rechargeWhen': 'Recharge when balance drops below'
    expect(screen.getByText('Recharge when balance drops below')).toBeInTheDocument();
    // en.ts: 'settings.billing.autoRecharge.addAmount': 'Add this amount'
    expect(screen.getByText('Add this amount')).toBeInTheDocument();
    // en.ts: 'settings.billing.autoRecharge.weeklyLimit': 'Weekly spending limit'
    expect(screen.getByText('Weekly spending limit')).toBeInTheDocument();
  });

  it('shows Save Settings button when dirty', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings({ enabled: true })}
        arDirty={true}
        arThreshold={5}
        arAmount={20}
      />
    );
    // en.ts: 'settings.billing.autoRecharge.saveSettings': 'Save Settings'
    expect(screen.getByRole('button', { name: 'Save Settings' })).toBeInTheDocument();
  });

  it('calls onArSave when Save is clicked', () => {
    const onArSave = vi.fn();
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings({ enabled: true })}
        arDirty={true}
        arThreshold={5}
        arAmount={20}
        onArSave={onArSave}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));
    expect(onArSave).toHaveBeenCalledTimes(1);
  });

  it('shows last error message from arSettings', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings({ enabled: true, lastError: 'Card declined' })}
      />
    );
    // en.ts: 'settings.billing.autoRecharge.lastRechargeFailed': 'Last recharge failed'
    expect(screen.getByText(/Last recharge failed/)).toBeInTheDocument();
    expect(screen.getByText(/Card declined/)).toBeInTheDocument();
  });

  it('shows recharge in-flight indicator', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings({ enabled: true, inFlight: true })}
      />
    );
    // en.ts: 'settings.billing.autoRecharge.rechargeInProgress': 'Recharge in progress'
    expect(screen.getByText('Recharge in progress')).toBeInTheDocument();
  });
});

describe('AutoRechargeSection — payment methods (lines 295, 372, 391)', () => {
  it('shows empty-state when no cards on file (line 295)', () => {
    render(<AutoRechargeSection {...defaultProps} arSettings={makeArSettings()} cards={[]} />);
    // en.ts: 'settings.billing.autoRecharge.noCards': 'No cards'
    expect(screen.getByText('No cards')).toBeInTheDocument();
  });

  it('renders cards loading skeleton when cardsLoading=true', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        cardsLoading={true}
        cards={[]}
      />
    );
    // No "No cards" text while loading
    expect(screen.queryByText('No cards')).not.toBeInTheDocument();
  });

  it('renders card with brand + last4', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        cards={[makeCard({ brand: 'visa', last4: '9999' })]}
      />
    );
    expect(screen.getByText(/Visa ••••9999/)).toBeInTheDocument();
  });

  it('renders "Set as default" button for non-default card (line 372)', () => {
    const onSetDefault = vi.fn();
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        cards={[makeCard({ isDefault: false })]}
        onSetDefault={onSetDefault}
      />
    );
    // en.ts: 'settings.billing.autoRecharge.setDefault': 'Set as default'
    const setDefaultBtn = screen.getByRole('button', { name: 'Set as default' });
    expect(setDefaultBtn).toBeInTheDocument();
    fireEvent.click(setDefaultBtn);
    expect(onSetDefault).toHaveBeenCalledWith('card-1');
  });

  it('does NOT render "Set as default" for default card', () => {
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        cards={[makeCard({ isDefault: true })]}
      />
    );
    expect(screen.queryByRole('button', { name: 'Set as default' })).not.toBeInTheDocument();
    // en.ts: 'settings.billing.autoRecharge.defaultCard': 'Default card'
    expect(screen.getByText('Default card')).toBeInTheDocument();
  });

  it('shows confirm+cancel when card is in confirmDelete state (line 391)', () => {
    const setConfirmDeleteId = vi.fn();
    const onDeleteCard = vi.fn();
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        cards={[makeCard()]}
        confirmDeleteId="card-1"
        setConfirmDeleteId={setConfirmDeleteId}
        onDeleteCard={onDeleteCard}
      />
    );
    // Confirm + Cancel buttons visible (en.ts: 'common.confirm', 'common.cancel')
    const confirmBtn = screen.getByRole('button', { name: /confirm/i });
    const cancelBtn = screen.getByRole('button', { name: /cancel/i });
    expect(confirmBtn).toBeInTheDocument();
    expect(cancelBtn).toBeInTheDocument();

    // Cancel clears confirmDeleteId
    fireEvent.click(cancelBtn);
    expect(setConfirmDeleteId).toHaveBeenCalledWith(null);

    // Confirm calls onDeleteCard
    fireEvent.click(confirmBtn);
    expect(onDeleteCard).toHaveBeenCalledWith('card-1');
  });

  it('clicking Remove sets the confirm state', () => {
    const setConfirmDeleteId = vi.fn();
    render(
      <AutoRechargeSection
        {...defaultProps}
        arSettings={makeArSettings()}
        cards={[makeCard()]}
        confirmDeleteId={null}
        setConfirmDeleteId={setConfirmDeleteId}
      />
    );
    // en.ts: 'common.remove': should be 'Remove'
    const removeBtn = screen.getByRole('button', { name: /remove/i });
    fireEvent.click(removeBtn);
    expect(setConfirmDeleteId).toHaveBeenCalledWith('card-1');
  });
});
