/*
 * "Add a provider" — a modal holding one rich select per provider category.
 *
 * WHY A MODAL. There are ~15 providers and a user connects one or two. Listing
 * all fifteen inline spends the section on the ones nobody chose and buries the
 * ones actually configured, so the panel lists only what is connected and this
 * dialog owns the catalogue.
 *
 * WHY THREE SELECTS AND NOT ONE GROUPED LIST. The categories are not three
 * slices of one decision, they are three different questions: a cloud provider
 * wants an API key, a local runtime wants an endpoint on this machine, a CLI
 * login wants nothing because another tool already holds the credential. One
 * flat list makes the user infer that from the group heading alone; a select
 * per category has a label and a line of helper text to say it outright, and
 * each collapses to a single row when it has nothing to offer.
 *
 * Custom is deliberately NOT a fourth select. It is one option, and a select
 * over one option is a button wearing a costume.
 *
 * WHY THE POPUP LEAVES THE MODAL. Radix portals `SelectContent` to the body, so
 * the list is not clipped by the dialog and a long cloud list is not squeezed
 * into whatever height is left below its trigger. The dialog and the popup are
 * both `z-50` from their own primitives and both portal to the body, which
 * leaves paint order to DOM order — true today, but only incidentally. The
 * explicit `z-60` makes the popup win by intent rather than by luck.
 */
import { useT } from '../../../../lib/i18n/I18nContext';
import Button from '../../../ui/Button';
import Label from '../../../ui/Label';
import { ModalShell } from '../../../ui/ModalShell';
import {
  SelectContent,
  SelectItem,
  SelectRoot,
  SelectTrigger,
  SelectValue,
} from '../../../ui/Select';
import { ProviderSwatch } from './ProviderListRow';

export interface ProviderOption {
  slug: string;
  label: string;
  tone: string;
  /** Second line: the endpoint host for a cloud provider, a short line about
   *  where it runs for the others. Deliberately not a translated per-provider
   *  blurb, which would be ~15 strings per locale to say what the host says. */
  detail: string;
}

export interface ProviderCategory {
  id: string;
  /** Already-translated field label, placeholder and helper line. */
  title: string;
  placeholder: string;
  helper: string;
  /** Only what is NOT yet connected. The panel behind the dialog shows the
   *  rest, and offering to add something twice is how you get two rows for one
   *  provider. */
  options: ProviderOption[];
}

const OptionSwatch = ({ slug, label, tone }: { slug: string; label: string; tone: string }) => {
  return <ProviderSwatch slug={slug} label={label} tone={tone} />;
};

const CategorySelect = ({
  category,
  onPick,
}: {
  category: ProviderCategory;
  onPick: (slug: string) => void;
}) => {
  const { t } = useT();
  const empty = category.options.length === 0;
  const triggerId = `add-provider-${category.id}`;

  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={triggerId} className="text-xs text-content-secondary">
        {category.title}
      </Label>

      {/* `value` is pinned to '' so the trigger always shows its placeholder:
          choosing an item starts a connect flow and leaves nothing selected
          here. A select that kept the last pick would claim a selection this
          component does not own — the connection state lives in the panel. */}
      <SelectRoot value="" onValueChange={onPick}>
        <SelectTrigger
          id={triggerId}
          inputSize="sm"
          disabled={empty}
          data-testid={`add-provider-select-${category.id}`}>
          <SelectValue
            placeholder={
              empty ? t('settings.ai.providers.categoryAllConnected') : category.placeholder
            }
          />
        </SelectTrigger>
        <SelectContent className="z-60 w-[min(24rem,calc(100vw-2rem))]">
          {category.options.map(option => (
            <SelectItem
              key={option.slug}
              value={option.slug}
              data-testid={`add-provider-option-${option.slug}`}>
              <span className="flex min-w-0 items-center gap-2">
                <OptionSwatch slug={option.slug} label={option.label} tone={option.tone} />
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-sm text-content">{option.label}</span>
                  <span className="truncate font-mono text-[10px] leading-3 text-content-muted">
                    {option.detail}
                  </span>
                </span>
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </SelectRoot>

      <p className="text-[11px] leading-4 text-content-muted">{category.helper}</p>
    </div>
  );
};

export const AddProviderDialog = ({
  categories,
  onPick,
  onAddCustom,
  onClose,
}: {
  categories: ProviderCategory[];
  /** Called with the chosen slug. The caller closes this dialog and starts the
   *  provider's own connect flow. */
  onPick: (slug: string) => void;
  /** Opens the full editor for a user-defined endpoint. */
  onAddCustom: () => void;
  onClose: () => void;
}) => {
  const { t } = useT();

  return (
    <ModalShell
      title={t('settings.ai.providers.addProvider')}
      titleId="add-provider-dialog-title"
      subtitle={t('settings.ai.providers.addProviderSubtitle')}
      onClose={onClose}
      maxWidthClassName="max-w-md">
      <div className="flex flex-col gap-4">
        {categories.map(category => (
          <CategorySelect key={category.id} category={category} onPick={onPick} />
        ))}

        {/* Not a fourth select: one option. */}
        <div className="flex flex-col gap-1.5 border-t border-line-subtle pt-4">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="self-start"
            onClick={onAddCustom}
            data-testid="add-provider-custom">
            {t('settings.ai.providers.addCustomAction')}
          </Button>
          <p className="text-[11px] leading-4 text-content-muted">
            {t('settings.ai.providers.customDetail')}
          </p>
        </div>
      </div>
    </ModalShell>
  );
};

export default AddProviderDialog;
