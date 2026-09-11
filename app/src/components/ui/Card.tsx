import { type ReactNode } from 'react';

import { cn } from '../../lib/cn';

export interface CardProps {
  title?: string;
  description?: string;
  /**
   * Right-hand side of the heading row — the `flex items-baseline
   * justify-between` header that a `title: string` cannot express on its own
   * (a UTC note, a range picker, a "view all" link). Rendering it switches the
   * heading block to a two-column baseline row; with it absent the heading
   * markup is untouched.
   */
  headerRight?: ReactNode;
  children: ReactNode;
  /**
   * Pad the body with `p-4`. Defaults to **false** because the ~60 existing
   * call sites (direct plus the `settings/controls/SettingsSection` shim) all
   * pad their own children — rows, `Alert`s, tables — so a `true` default
   * would add a gutter to every one of them. New call sites that are
   * reproducing the hand-rolled `rounded-xl border bg-surface p-4` recipe
   * should pass `padded`.
   */
  padded?: boolean;
  /**
   * Separate direct children with `divide-y divide-line-subtle`. Defaults to
   * **true**, which is what the body has always done; a card holding a single
   * block rather than a list of rows wants `divided={false}`.
   */
  divided?: boolean;
  className?: string;
  'data-testid'?: string;
}

/**
 * A bordered surface with an optional heading and divided body — the shape
 * ~470 hand-rolled `rounded-* border bg-*` wrappers across the app are
 * reproducing. Generalized out of `settings/controls/SettingsSection`, which
 * now re-exports this.
 *
 * `padded` / `divided` / `headerRight` are all additive: the no-prop render is
 * byte-identical to the version that had none of them, so no existing call
 * site had to change when they landed.
 */
const Card = ({
  title,
  description,
  headerRight,
  children,
  padded = false,
  divided = true,
  className,
  'data-testid': testId,
}: CardProps) => {
  /* Real heading (h3, one level below SettingsHeader's h2) for a11y and so
     getByRole('heading') keeps resolving section titles. */
  const heading = title ? (
    <>
      <h3 className="text-xs font-semibold tracking-wide text-content-muted">{title}</h3>
      {description && (
        <p className="mt-1 text-xs leading-relaxed text-content-muted">{description}</p>
      )}
    </>
  ) : null;

  return (
    <div
      data-slot="card"
      data-testid={testId}
      className={cn('overflow-hidden rounded-xl border border-line bg-surface', className)}>
      {(heading || headerRight) && (
        <div className="px-4 pb-0 pt-4">
          {headerRight ? (
            <div className="flex items-baseline justify-between gap-3">
              <div className="min-w-0">{heading}</div>
              <div className="shrink-0">{headerRight}</div>
            </div>
          ) : (
            heading
          )}
        </div>
      )}
      {/* `divide-line-subtle` flips with the theme on its own, so the historical
          hardcoded dark-mode companion is gone: a raw palette scale would not
          follow a user's custom theme. */}
      <div className={cn(divided && 'divide-y divide-line-subtle', padded && 'p-4')}>
        {children}
      </div>
    </div>
  );
};

export default Card;
