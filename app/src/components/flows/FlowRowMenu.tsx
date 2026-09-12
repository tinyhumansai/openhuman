/**
 * FlowRowMenu — the "⋯" overflow menu on a Workflows list row. Holds the
 * secondary/rare actions (Export, Duplicate, Delete) so the row's primary
 * actions (View runs, Run) stay uncluttered, and keeps the destructive Delete
 * out of the flat button row. Closes on Escape, outside click, or item select
 * — all handled by Radix `DropdownMenu` (`../ui`), which also
 * portals the content so it escapes the list card's `overflow-hidden`
 * clipping and flips placement when there isn't room in the viewport.
 *
 * Presentational + local open state only — each item calls back up to
 * `FlowListRow`, which routes to `FlowsPage`'s handlers.
 */
import { useT } from '../../lib/i18n/I18nContext';
import {
  Button,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from '../ui';

interface FlowRowMenuItem {
  key: string;
  label: string;
  onSelect: () => void;
  /** Renders the item in the destructive coral tone (e.g. Delete). */
  danger?: boolean;
  testId?: string;
}

interface FlowRowMenuProps {
  items: FlowRowMenuItem[];
  /** Suffixed onto test ids so multiple rows stay addressable. */
  rowId: string;
}

function KebabIcon() {
  return (
    <svg className="h-4 w-4" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <circle cx="12" cy="5" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="12" cy="19" r="1.6" />
    </svg>
  );
}

export default function FlowRowMenu({ items, rowId }: FlowRowMenuProps) {
  const { t } = useT();

  return (
    <DropdownMenuRoot>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          iconOnly
          data-testid={`flow-menu-${rowId}`}
          aria-label={t('flows.list.moreActions')}
          title={t('flows.list.moreActions')}
          className="rounded-lg">
          <KebabIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent data-testid={`flow-menu-list-${rowId}`}>
        {items.map(item => (
          <DropdownMenuItem
            key={item.key}
            data-testid={item.testId}
            onSelect={item.onSelect}
            className={item.danger ? 'text-coral-600 dark:text-coral-400' : undefined}>
            {item.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenuRoot>
  );
}
