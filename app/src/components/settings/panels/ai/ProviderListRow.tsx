/*
 * The provider list's row and group primitives.
 *
 * WHY A LIST AND NOT CHIPS. Providers used to render as one flex-wrap row of
 * pills, each pill a name plus a `Switch`. That shape carries exactly one fact
 * per provider — on or off — so everything else the user needs had to live
 * somewhere else: whether a key is stored, which endpoint a local runtime
 * points at, why "connected" and "actually in use" can disagree. Reading the
 * wrap order also gave no hint that OpenAI, Ollama and a CLI login are three
 * different KINDS of thing, and the two CLI logins ended up as loose buttons
 * below the pills because they never fit the pill shape at all.
 *
 * A row has room for a secondary line, so each provider states its own status
 * where the user is already looking, and the groups name the distinction the
 * wrap order could not.
 *
 * THE SWITCH IS THE COMPATIBILITY CONTRACT. Connect/disconnect stays a
 * `Switch` carrying `providerToggleAriaLabel`'s "Connect X" / "Disconnect X"
 * accessible name, unchanged from the chips. That name is what the panel's
 * tests drive and what a screen-reader user already knows; the layout around
 * it is free to change, that name is not.
 */
import { type ReactNode } from 'react';
import { LuEllipsisVertical } from 'react-icons/lu';

import { cn } from '../../../../lib/cn';
import { useT } from '../../../../lib/i18n/I18nContext';
import Button from '../../../ui/Button';
import Card from '../../../ui/Card';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from '../../../ui/DropdownMenu';
import { providerIcon } from './providerIcons';

/** One labelled band of the list. The heading names what the rows have in
 *  common, which is the thing a wrapped pill row could not express. */
export const ProviderGroup = ({
  title,
  children,
  card = false,
  'data-testid': testId,
}: {
  title: string;
  children: ReactNode;
  /** Use the shared Card primitive when this group is a top-level section. */
  card?: boolean;
  'data-testid'?: string;
}) => {
  const list = <ul className="flex w-full flex-col divide-y divide-line-subtle">{children}</ul>;

  return card ? (
    <Card title={title} data-testid={testId} className="w-full">
      {list}
    </Card>
  ) : (
    <section className="flex w-full flex-col" data-testid={testId}>
      <h4 className="px-4 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
        {title}
      </h4>
      <div className="border-y border-line-subtle">{list}</div>
    </section>
  );
};

export interface ProviderRowAction {
  label: string;
  onSelect: () => void;
  /** Renders in the danger tone — removal, key clearing. */
  destructive?: boolean;
}

const PROVIDER_SWATCH_TONES: Record<string, string> = {
  openai: 'bg-[#000000] text-content-inverted ring-content-inverted/30',
  anthropic: 'bg-[#D97757] text-content-inverted ring-content-inverted/30',
  'claude-code': 'bg-[#D97757] text-content-inverted ring-content-inverted/30',
  google: 'bg-[#4285F4] text-content-inverted ring-content-inverted/30',
  mistral: 'bg-[#FF7000] text-content-inverted ring-content-inverted/30',
  huggingface: 'bg-[#B88600] text-content-inverted ring-content-inverted/30',
  nvidia: 'bg-[#5A8F00] text-content-inverted ring-content-inverted/30',
  'vercel-ai-gateway': 'bg-[#000000] text-content-inverted ring-content-inverted/30',
  xai: 'bg-[#000000] text-content-inverted ring-content-inverted/30',
  zai: 'bg-[#FF6A00] text-content-inverted ring-content-inverted/30',
  ollama: 'bg-[#000000] text-content-inverted ring-content-inverted/30',
  lmstudio: 'bg-[#303E5C] text-content-inverted ring-content-inverted/30',
  omlx: 'bg-[#6E6E73] text-content-inverted ring-content-inverted/30',
  gmi: 'bg-[#0F766E] text-content-inverted ring-content-inverted/30',
  openrouter: 'bg-[#7624F4] text-content-inverted ring-content-inverted/30',
  deepseek: 'bg-[#4D6BFE] text-content-inverted ring-content-inverted/30',
  cerebras: 'bg-[#FF6A00] text-content-inverted ring-content-inverted/30',
  fireworks: 'bg-[#EF476F] text-content-inverted ring-content-inverted/30',
  groq: 'bg-[#F55036] text-content-inverted ring-content-inverted/30',
  minimax: 'bg-[#7048E8] text-content-inverted ring-content-inverted/30',
  moonshot: 'bg-[#5B5CE2] text-content-inverted ring-content-inverted/30',
  together: 'bg-[#9D36D5] text-content-inverted ring-content-inverted/30',
};

/**
 * The provider's brand mark on its tone swatch, falling back to the initial.
 *
 * The letter is not a placeholder to be embarrassed about: Simple Icons covers
 * roughly a third of the providers here, and drawing approximations of the
 * other companies' logos would be worse than a letter that is at least
 * unambiguous. `aria-hidden` either way, because the name is right beside it —
 * announcing "O, OpenAI" helps nobody.
 */
export const ProviderSwatch = ({
  slug,
  label,
}: {
  slug: string;
  label: string;
  /** Retained for callers that supply a custom-provider fallback tone. */
  tone: string;
}) => {
  const icon = providerIcon(slug, 'h-4 w-4');
  return (
    <span
      aria-hidden
      data-slot="provider-swatch"
      className={cn(
        'flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-xs font-semibold ring-1',
        PROVIDER_SWATCH_TONES[slug] ?? 'bg-[#27272A] text-content-inverted ring-content-inverted/30'
      )}>
      {icon ?? (label.trim().charAt(0).toUpperCase() || '?')}
    </span>
  );
};

export const ProviderListRow = ({
  slug,
  label,
  tone,
  detail,
  detailMono = false,
  badge,
  control,
  actions = [],
  actionsLabel,
  'data-testid': testId,
}: {
  /** Provider slug, used to look up the brand mark. */
  slug: string;
  label: string;
  tone: string;
  /** Secondary line: a masked key, an endpoint, or a connection state. */
  detail: ReactNode;
  /** Endpoints and masked keys are strings to compare character by character,
   *  so they get a mono face; prose does not. */
  detailMono?: boolean;
  badge?: ReactNode;
  /** The row's primary control — a `Switch`, or a status badge for a provider
   *  that cannot be turned off. */
  control: ReactNode;
  /** Secondary actions, collapsed into an overflow menu. Empty renders no
   *  trigger at all rather than a menu with nothing in it. */
  actions?: ProviderRowAction[];
  /** Accessible name for the overflow trigger. Required when `actions` is
   *  non-empty — an unnamed icon button is announced as just "button". */
  actionsLabel?: string;
  'data-testid'?: string;
}) => {
  const { t } = useT();
  return (
    <li
      data-slot="provider-row"
      data-testid={testId}
      className="flex items-center gap-3 px-4 py-2.5 transition-colors hover:bg-surface-hover">
      <ProviderSwatch slug={slug} label={label} tone={tone} />

      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-medium text-content">{label}</span>
          {badge}
        </div>
        <span
          className={cn(
            'truncate text-[11px] leading-4 text-content-muted',
            detailMono && 'font-mono'
          )}>
          {detail}
        </span>
      </div>

      <div className="flex shrink-0 items-center gap-1">
        {control}
        {actions.length > 0 && (
          <DropdownMenuRoot>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                iconOnly
                variant="tertiary"
                size="xs"
                aria-label={actionsLabel ?? t('common.edit')}
                title={actionsLabel}>
                <LuEllipsisVertical className="h-3.5 w-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              {actions.map(action => (
                <DropdownMenuItem
                  key={action.label}
                  onSelect={action.onSelect}
                  className={cn(action.destructive && 'text-coral-600 dark:text-coral-300')}>
                  {action.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenuRoot>
        )}
      </div>
    </li>
  );
};

export default ProviderListRow;
