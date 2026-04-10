import type { AutoRechargeSettings, SavedCard } from '../../../../services/api/creditsApi';

// ── Constants ────────────────────────────────────────────────────────────────
const THRESHOLD_OPTIONS = [5, 10, 20] as const;
const RECHARGE_OPTIONS = [10, 20, 50, 100] as const;
const WEEKLY_LIMIT_OPTIONS = [25, 50, 100, 200, 500] as const;

const CARD_BRAND_LABELS: Record<string, string> = {
  visa: 'Visa',
  mastercard: 'Mastercard',
  amex: 'Amex',
  discover: 'Discover',
  jcb: 'JCB',
  diners: 'Diners',
  unionpay: 'UnionPay',
};

function cardBrandLabel(brand: string) {
  return CARD_BRAND_LABELS[brand.toLowerCase()] ?? brand.charAt(0).toUpperCase() + brand.slice(1);
}

interface AutoRechargeSectionProps {
  arSettings: AutoRechargeSettings | null;
  arLoading: boolean;
  arError: string | null;
  arSaving: boolean;
  arThreshold: number;
  arAmount: number;
  arWeeklyLimit: number;
  arDirty: boolean;
  setArThreshold: (v: number) => void;
  setArAmount: (v: number) => void;
  setArWeeklyLimit: (v: number) => void;
  setArError: (v: string | null) => void;
  onArToggle: () => void;
  onArSave: () => void;
  // Cards
  cards: SavedCard[];
  cardsLoading: boolean;
  confirmDeleteId: string | null;
  deletingCardId: string | null;
  settingDefaultId: string | null;
  setConfirmDeleteId: (v: string | null) => void;
  onSetDefault: (paymentMethodId: string) => void;
  onDeleteCard: (paymentMethodId: string) => void;
  onAddCard: () => void;
}

const AutoRechargeSection = ({
  arSettings,
  arLoading,
  arError,
  arSaving,
  arThreshold,
  arAmount,
  arWeeklyLimit,
  arDirty,
  setArThreshold,
  setArAmount,
  setArWeeklyLimit,
  setArError,
  onArToggle,
  onArSave,
  cards,
  cardsLoading,
  confirmDeleteId,
  deletingCardId,
  settingDefaultId,
  setConfirmDeleteId,
  onSetDefault,
  onDeleteCard,
  onAddCard,
}: AutoRechargeSectionProps) => (
  <div className="rounded-2xl border border-stone-200 bg-white overflow-hidden">
    {/* Header row */}
    <div className="flex items-center justify-between p-3">
      <div>
        <p className="text-xs font-semibold text-stone-900">Auto-Recharge Credits</p>
        <p className="text-[11px] text-stone-400 mt-0.5">
          Automatically top up when your balance runs low
        </p>
      </div>
      {arLoading ? (
        <div className="w-10 h-5 rounded-full bg-stone-700/60 animate-pulse" />
      ) : (
        <button
          onClick={onArToggle}
          disabled={arSaving}
          role="switch"
          aria-checked={arSettings?.enabled ?? false}
          aria-label="Toggle auto-recharge"
          className={`relative w-10 h-5 rounded-full transition-colors focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-2 focus-visible:ring-offset-stone-900 ${
            arSaving ? 'opacity-50 cursor-not-allowed' : ''
          } ${arSettings?.enabled ? 'bg-primary-500' : 'bg-stone-600'}`}>
          <span
            className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
              arSettings?.enabled ? 'translate-x-5' : 'translate-x-0'
            }`}
          />
        </button>
      )}
    </div>

    {/* Error banner */}
    {arError && (
      <div className="mx-3 mb-2 flex items-start gap-2 rounded-lg bg-coral-500/10 border border-coral-500/20 px-2.5 py-2">
        <svg
          className="w-3.5 h-3.5 text-coral-400 flex-shrink-0 mt-0.5"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
          />
        </svg>
        <p className="text-[11px] text-coral-300 leading-relaxed">{arError}</p>
        <button
          onClick={() => setArError(null)}
          className="ml-auto text-coral-400 hover:text-coral-300 flex-shrink-0"
          aria-label="Dismiss error">
          <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>
    )}

    {/* Settings — only shown when enabled */}
    {!arLoading && arSettings?.enabled && (
      <div className="border-t border-stone-200 px-3 pt-3 pb-2 space-y-3">
        {/* Status row */}
        <div className="flex items-center gap-3 flex-wrap">
          {arSettings.inFlight && (
            <span className="flex items-center gap-1 text-[10px] text-amber-700 bg-amber-50 border border-amber-200 rounded-full px-2 py-0.5">
              <svg className="w-2.5 h-2.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
              Recharge in progress
            </span>
          )}
          {arSettings.spentThisWeekUsd > 0 && (
            <span className="text-[10px] text-stone-400">
              ${arSettings.spentThisWeekUsd.toFixed(2)} of ${arSettings.weeklyLimitUsd} used this
              week
            </span>
          )}
          {arSettings.lastRechargeAt && (
            <span className="text-[10px] text-stone-500">
              Last recharged{' '}
              {new Date(arSettings.lastRechargeAt).toLocaleDateString('en-US', {
                month: 'short',
                day: 'numeric',
              })}
            </span>
          )}
        </div>

        {/* Last error from recharge attempt */}
        {arSettings.lastError && (
          <div className="flex items-start gap-1.5 rounded-lg bg-coral-500/10 border border-coral-500/20 px-2.5 py-2">
            <svg
              className="w-3 h-3 text-coral-400 flex-shrink-0 mt-0.5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
              />
            </svg>
            <p className="text-[10px] text-coral-300">
              Last recharge failed: {arSettings.lastError}
            </p>
          </div>
        )}

        {/* Trigger threshold */}
        <div>
          <p className="text-[11px] text-stone-400 mb-1.5">Recharge when balance drops below</p>
          <div className="flex gap-1.5 flex-wrap">
            {THRESHOLD_OPTIONS.map(v => (
              <button
                key={v}
                onClick={() => setArThreshold(v)}
                className={`px-2.5 py-1 text-xs rounded-lg border transition-colors ${
                  arThreshold === v
                    ? 'bg-primary-500/20 text-primary-400 border-primary-500/40'
                    : 'bg-stone-100 text-stone-500 border-stone-200 hover:text-stone-700'
                }`}>
                ${v}
              </button>
            ))}
          </div>
        </div>

        {/* Recharge amount */}
        <div>
          <p className="text-[11px] text-stone-400 mb-1.5">Add this amount</p>
          <div className="flex gap-1.5 flex-wrap">
            {RECHARGE_OPTIONS.map(v => (
              <button
                key={v}
                onClick={() => setArAmount(v)}
                className={`px-2.5 py-1 text-xs rounded-lg border transition-colors ${
                  arAmount === v
                    ? 'bg-primary-500/20 text-primary-400 border-primary-500/40'
                    : 'bg-stone-100 text-stone-500 border-stone-200 hover:text-stone-700'
                }`}>
                ${v}
              </button>
            ))}
          </div>
        </div>

        {/* Weekly limit */}
        <div>
          <p className="text-[11px] text-stone-400 mb-1.5">Weekly spending limit</p>
          <div className="flex gap-1.5 flex-wrap">
            {WEEKLY_LIMIT_OPTIONS.map(v => (
              <button
                key={v}
                onClick={() => setArWeeklyLimit(v)}
                className={`px-2.5 py-1 text-xs rounded-lg border transition-colors ${
                  arWeeklyLimit === v
                    ? 'bg-primary-500/20 text-primary-400 border-primary-500/40'
                    : 'bg-stone-100 text-stone-500 border-stone-200 hover:text-stone-700'
                }`}>
                ${v}
              </button>
            ))}
          </div>
        </div>

        {/* Validation hint */}
        {arAmount <= arThreshold && (
          <p className="text-[10px] text-amber-400">
            Recharge amount should be greater than the trigger threshold.
          </p>
        )}

        {/* Save button */}
        {arDirty && (
          <button
            onClick={onArSave}
            disabled={arSaving || arAmount <= arThreshold}
            className={`w-full py-1.5 text-xs font-medium rounded-lg transition-colors ${
              arSaving || arAmount <= arThreshold
                ? 'bg-stone-700/40 text-stone-500 cursor-not-allowed'
                : 'bg-primary-500 hover:bg-primary-600 text-white'
            }`}>
            {arSaving ? 'Saving…' : 'Save Settings'}
          </button>
        )}
      </div>
    )}

    {/* Payment methods */}
    <div className="border-t border-stone-200 px-3 py-2.5">
      <div className="flex items-center justify-between mb-2">
        <p className="text-[11px] font-medium text-stone-600">Payment Methods</p>
        <button
          onClick={onAddCard}
          className="text-[11px] text-primary-400 hover:text-primary-300 font-medium transition-colors">
          + Add card
        </button>
      </div>

      {cardsLoading ? (
        <div className="space-y-1.5">
          {[0, 1].map(i => (
            <div key={i} className="h-9 rounded-lg bg-stone-700/30 animate-pulse" />
          ))}
        </div>
      ) : cards.length === 0 ? (
        <div className="flex items-center gap-2 rounded-lg bg-stone-50 border border-stone-200 p-2.5">
          <svg
            className="w-4 h-4 text-stone-500 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.5}
              d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z"
            />
          </svg>
          <p className="text-[11px] text-stone-500">
            No saved cards. Add one to enable auto-recharge.
          </p>
        </div>
      ) : (
        <div className="space-y-1.5">
          {cards.map(card => {
            const isDeleting = deletingCardId === card.id;
            const isSettingDefault = settingDefaultId === card.id;
            const isConfirming = confirmDeleteId === card.id;

            return (
              <div
                key={card.id}
                className="flex items-center gap-2 rounded-lg bg-stone-50 border border-stone-200 px-2.5 py-2">
                {/* Card icon */}
                <svg
                  className="w-4 h-4 text-stone-400 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.5}
                    d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z"
                  />
                </svg>

                {/* Card info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <span className="text-xs text-stone-900 font-medium">
                      {cardBrandLabel(card.brand)} ••••{card.last4}
                    </span>
                    {card.isDefault && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-primary-500/20 text-primary-400 border border-primary-500/30 font-medium">
                        Default
                      </span>
                    )}
                  </div>
                  <p className="text-[10px] text-stone-500 mt-0.5">
                    Expires {String(card.expMonth).padStart(2, '0')}/
                    {String(card.expYear).slice(-2)}
                  </p>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-1 flex-shrink-0">
                  {!card.isDefault && (
                    <button
                      onClick={() => onSetDefault(card.id)}
                      disabled={!!settingDefaultId || !!deletingCardId}
                      className="text-[10px] text-stone-500 hover:text-stone-700 transition-colors disabled:opacity-40 disabled:cursor-not-allowed px-1.5 py-1">
                      {isSettingDefault ? '…' : 'Set default'}
                    </button>
                  )}

                  {isConfirming ? (
                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => onDeleteCard(card.id)}
                        disabled={isDeleting}
                        className="text-[10px] text-coral-400 hover:text-coral-300 font-medium transition-colors disabled:opacity-40 px-1.5 py-1">
                        {isDeleting ? '…' : 'Confirm'}
                      </button>
                      <button
                        onClick={() => setConfirmDeleteId(null)}
                        className="text-[10px] text-stone-500 hover:text-stone-400 transition-colors px-1 py-1">
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <button
                      onClick={() => setConfirmDeleteId(card.id)}
                      disabled={isDeleting || !!settingDefaultId}
                      className="text-[10px] text-stone-500 hover:text-coral-400 transition-colors disabled:opacity-40 disabled:cursor-not-allowed px-1.5 py-1">
                      Remove
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  </div>
);

export default AutoRechargeSection;
