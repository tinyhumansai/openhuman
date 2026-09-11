import { cva, type VariantProps } from 'class-variance-authority';

/**
 * Named content-width scale for the layout primitives that center a column of
 * page content (`PageSectionHeader`, `PageWelcome`, and — opt-in — `PanelPage`
 * / `PanelScaffold` bodies).
 *
 * Audited app-wide before adding this: `max-w-md` ×28, `max-w-sm` ×18,
 * `max-w-3xl` ×18, `max-w-2xl` ×12, `max-w-lg` ×11, `max-w-xl` ×8,
 * `max-w-5xl` ×6, `max-w-xs` ×5, plus 30+ arbitrary `max-w-[…]` values — every
 * caller picking its own guess rather than a shared scale. This file does not
 * attempt to migrate all of them (most live outside `components/layout/`, and
 * many — a truncating label, a modal body — are legitimately local rather than
 * a page-width decision). It establishes the scale for the primitives owned
 * here; see the migration note in the PR description for the rest.
 *
 * Four sizes, chosen from what the audited primitives actually needed:
 * - `sm` — a narrow single-column read (form, empty state).
 * - `md` — the `PageWelcome` pitch width (today's hardcoded `max-w-2xl`).
 * - `lg` — a wider dashboard-style header/body (today's `max-w-3xl` callers).
 * - `full` — no constraint (the default everywhere except `PageWelcome`,
 *   which centers by design).
 */
export const contentWidthVariants = cva('', {
  variants: { width: { sm: 'max-w-lg', md: 'max-w-2xl', lg: 'max-w-3xl', full: '' } },
  defaultVariants: { width: 'full' },
});

export type ContentWidth = NonNullable<VariantProps<typeof contentWidthVariants>['width']>;
