import { useT } from '../../../../lib/i18n/I18nContext';
import type { AutoRechargeSettings, SavedCard } from '../../../../services/api/creditsApi';
import { Alert, AlertDescription } from '../../../ui/Alert';
import Badge from '../../../ui/Badge';
import Button from '../../../ui/Button';
import Card from '../../../ui/Card';
import { WarningIcon } from '../../../ui/icons';
import Switch from '../../../ui/Switch';
import { ToggleGroupItem, ToggleGroupRoot } from '../../../ui/ToggleGroup';

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

/** A single-select row of dollar-amount pills backed by `ToggleGroup`. */
function AmountPicker({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly number[];
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <div>
      <p className="text-[11px] text-content-faint mb-1.5">{label}</p>
      <ToggleGroupRoot
        type="single"
        variant="secondary"
        size="xs"
        value={String(value)}
        onValueChange={next => {
          if (next) onChange(Number(next));
        }}
        className="flex-wrap">
        {options.map(v => (
          <ToggleGroupItem key={v} value={String(v)}>
            ${v}
          </ToggleGroupItem>
        ))}
      </ToggleGroupRoot>
    </div>
  );
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
}: AutoRechargeSectionProps) => {
  const { t } = useT();
  return (
    <Card className="rounded-2xl">
      {/* Header row */}
      <div className="flex items-center justify-between p-3">
        <div>
          <p className="text-sm font-semibold text-content">
            {t('settings.billing.autoRecharge.title')}
          </p>
          <p className="text-[11px] text-content-faint mt-0.5">
            {t('settings.billing.autoRecharge.subtitle')}
          </p>
        </div>
        {arLoading ? (
          <div className="w-10 h-5 rounded-full bg-surface-strong animate-pulse" />
        ) : (
          <Switch
            id="auto-recharge-toggle"
            checked={arSettings?.enabled ?? false}
            onCheckedChange={onArToggle}
            disabled={arSaving}
            aria-label={t('settings.billing.autoRecharge.toggleAriaLabel')}
          />
        )}
      </div>

      {/* Error banner */}
      {arError && (
        <Alert variant="destructive" className="mx-3 mb-2 items-start rounded-lg py-2">
          <WarningIcon className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <AlertDescription className="text-[12px] leading-relaxed opacity-100">
            {arError}
          </AlertDescription>
        </Alert>
      )}

      {/* Settings — only shown when enabled */}
      {!arLoading && arSettings?.enabled && (
        <div className="px-3 pt-3 pb-2 space-y-3">
          {/* Status row */}
          <div className="flex items-center gap-3 flex-wrap">
            {arSettings.inFlight && (
              <span className="flex items-center gap-1 text-[10px] text-amber-700 dark:text-amber-300 bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/30 rounded-full px-2 py-0.5">
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
                {t('settings.billing.autoRecharge.rechargeInProgress')}
              </span>
            )}
            {arSettings.spentThisWeekUsd > 0 && (
              <span className="text-[10px] text-content-faint">
                {t('settings.billing.autoRecharge.spentThisWeek')
                  .replace('{spent}', arSettings.spentThisWeekUsd.toFixed(2))
                  .replace('{limit}', String(arSettings.weeklyLimitUsd))}
              </span>
            )}
            {arSettings.lastRechargeAt && (
              <span className="text-[10px] text-content-muted">
                {t('settings.billing.autoRecharge.expires').replace(
                  '{date}',
                  new Date(arSettings.lastRechargeAt).toLocaleDateString('en-US', {
                    month: 'short',
                    day: 'numeric',
                  })
                )}
              </span>
            )}
          </div>

          {/* Last error from recharge attempt */}
          {arSettings.lastError && (
            <Alert variant="destructive" className="items-start rounded-lg py-2 gap-1.5">
              <WarningIcon className="w-3 h-3 shrink-0 mt-0.5" />
              <AlertDescription className="text-[10px] opacity-100">
                {t('settings.billing.autoRecharge.lastRechargeFailed')}: {arSettings.lastError}
              </AlertDescription>
            </Alert>
          )}

          {/* Trigger threshold / recharge amount / weekly limit — pill selectors */}
          <AmountPicker
            label={t('settings.billing.autoRecharge.rechargeWhen')}
            options={THRESHOLD_OPTIONS}
            value={arThreshold}
            onChange={setArThreshold}
          />
          <AmountPicker
            label={t('settings.billing.autoRecharge.addAmount')}
            options={RECHARGE_OPTIONS}
            value={arAmount}
            onChange={setArAmount}
          />
          <AmountPicker
            label={t('settings.billing.autoRecharge.weeklyLimit')}
            options={WEEKLY_LIMIT_OPTIONS}
            value={arWeeklyLimit}
            onChange={setArWeeklyLimit}
          />

          {/* Validation hint */}
          {arAmount <= arThreshold && (
            <p className="text-[10px] text-amber-400">
              {t('settings.billing.autoRecharge.amountHint')}
            </p>
          )}

          {/* Save button */}
          {arDirty && (
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={onArSave}
              disabled={arSaving || arAmount <= arThreshold}
              className="w-full">
              {arSaving
                ? t('settings.billing.autoRecharge.saving')
                : t('settings.billing.autoRecharge.saveSettings')}
            </Button>
          )}
        </div>
      )}

      {/* Payment methods */}
      <div className="px-3 py-2.5">
        <div className="flex items-center justify-between mb-2">
          <p className="text-[11px] font-medium text-content-secondary">
            {t('settings.billing.autoRecharge.paymentMethods')}
          </p>
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            onClick={onAddCard}
            className="text-primary-500 dark:text-primary-400 hover:text-primary-600 dark:hover:text-primary-300">
            {t('settings.billing.autoRecharge.addCard')}
          </Button>
        </div>

        {cardsLoading ? (
          <div className="space-y-1.5">
            {[0, 1].map(i => (
              <div key={i} className="h-9 rounded-lg bg-surface-strong/60 animate-pulse" />
            ))}
          </div>
        ) : cards.length === 0 ? (
          <div className="flex items-center gap-2 rounded-lg bg-surface-muted border border-line p-2.5">
            <svg
              className="w-4 h-4 text-content-muted shrink-0"
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
            <p className="text-[11px] text-content-muted">
              {t('settings.billing.autoRecharge.noCards')}
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
                  className="flex items-center gap-2 rounded-lg bg-surface-muted border border-line px-2.5 py-2">
                  {/* Card icon */}
                  <svg
                    className="w-4 h-4 text-content-faint shrink-0"
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
                      <span className="text-xs text-content font-medium">
                        {cardBrandLabel(card.brand)} ••••{card.last4}
                      </span>
                      {card.isDefault && (
                        <Badge variant="primary">
                          {t('settings.billing.autoRecharge.defaultCard')}
                        </Badge>
                      )}
                    </div>
                    <p className="text-[10px] text-content-muted mt-0.5">
                      {t('settings.billing.autoRecharge.expires').replace(
                        '{date}',
                        `${String(card.expMonth).padStart(2, '0')}/${String(card.expYear).slice(-2)}`
                      )}
                    </p>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-1 shrink-0">
                    {!card.isDefault && (
                      <Button
                        type="button"
                        variant="tertiary"
                        size="xs"
                        onClick={() => onSetDefault(card.id)}
                        disabled={!!settingDefaultId || !!deletingCardId}>
                        {isSettingDefault ? '…' : t('settings.billing.autoRecharge.setDefault')}
                      </Button>
                    )}

                    {isConfirming ? (
                      <div className="flex items-center gap-1">
                        <Button
                          type="button"
                          variant="primary"
                          tone="danger"
                          size="xs"
                          onClick={() => onDeleteCard(card.id)}
                          disabled={isDeleting}>
                          {isDeleting ? '…' : t('common.confirm')}
                        </Button>
                        <Button
                          type="button"
                          variant="tertiary"
                          size="xs"
                          onClick={() => setConfirmDeleteId(null)}>
                          {t('common.cancel')}
                        </Button>
                      </div>
                    ) : (
                      <Button
                        type="button"
                        variant="tertiary"
                        size="xs"
                        onClick={() => setConfirmDeleteId(card.id)}
                        disabled={isDeleting || !!settingDefaultId}
                        className="text-content-muted hover:text-coral-600 dark:hover:text-coral-400">
                        {t('common.remove')}
                      </Button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </Card>
  );
};

export default AutoRechargeSection;
