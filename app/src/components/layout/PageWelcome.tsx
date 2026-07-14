import type { ReactNode } from 'react';

import Button from '../ui/Button';

/**
 * PageWelcome — the welcome landing shown as the first destination of a sidebar
 * page. A compact, friendly pitch: an accent icon tile, an eyebrow + title, a
 * short lead paragraph on what the page is for, a few immediate CTAs into the
 * real views, and a small grid of benefit cards ("what you can do here").
 *
 * Deliberately restrained — no gradient hero. Pages render it as their default
 * view and wire each CTA to switch to the relevant functional tab.
 */

/** Accent families available for the icon tile. Keys map to design tokens. */
export type PageWelcomeAccent = 'ocean' | 'sage' | 'amber' | 'coral';

/** Static per-accent tile tint (Tailwind only scans literal class names). */
const ACCENT_TILE: Record<PageWelcomeAccent, string> = {
  ocean: 'bg-primary-500/15 text-primary-600',
  sage: 'bg-sage-500/15 text-sage-600',
  amber: 'bg-amber-500/15 text-amber-600',
  coral: 'bg-coral-500/15 text-coral-600',
};

export interface WelcomeFeature {
  /** Leading glyph — emoji string or icon node. */
  icon: ReactNode;
  /** Short feature title. */
  title: ReactNode;
  /** One- or two-line benefit description. */
  description: ReactNode;
}

export interface WelcomeCta {
  label: ReactNode;
  onClick: () => void;
  /** First CTA renders as primary; the rest as secondary (override here). */
  variant?: 'primary' | 'secondary';
  icon?: ReactNode;
  testId?: string;
}

export interface PageWelcomeProps {
  /** Big glyph in the accent tile — emoji string or icon node. */
  icon: ReactNode;
  /** Optional short lead-in above the title (e.g. the page name). */
  eyebrow?: ReactNode;
  /** Page welcome title — a benefit-led thesis, not just the page name. */
  title: ReactNode;
  /** Lead paragraph: what the page is for, in plain language. */
  description: ReactNode;
  /** A few immediate actions into the functional views (first = primary). */
  ctas?: WelcomeCta[];
  /** Heading above the feature grid (e.g. "What you can do here"). */
  featuresHeading?: ReactNode;
  /** Benefit cards. */
  features?: WelcomeFeature[];
  accent?: PageWelcomeAccent;
  testId?: string;
}

export default function PageWelcome({
  icon,
  eyebrow,
  title,
  description,
  ctas,
  featuresHeading,
  features,
  accent = 'ocean',
  testId,
}: PageWelcomeProps) {
  const tile = ACCENT_TILE[accent];
  const tint = ACCENT_TILE[accent].split(' ')[0]; // just the bg-*/15 tint

  return (
    // Vertically center the pitch; when it's taller than the viewport the outer
    // scroll takes over and the top stays reachable (min-h-full, not h-full).
    <div className="h-full overflow-y-auto">
      <div className="flex min-h-full items-center">
        <div data-testid={testId} className="mx-auto w-full max-w-2xl animate-fade-up px-6 py-10">
          <div
            aria-hidden
            className={`mb-5 flex items-center justify-center rounded-2xl text-3xl ${tile}`}
            style={{ height: '3.75rem', width: '3.75rem' }}>
            {icon}
          </div>

          {eyebrow != null && (
            <p className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-content-muted">
              {eyebrow}
            </p>
          )}
          <h1 className="font-title text-2xl font-semibold tracking-tight text-content">{title}</h1>
          <p className="mt-2.5 max-w-xl text-sm leading-relaxed text-content-secondary">
            {description}
          </p>

          {ctas != null && ctas.length > 0 && (
            <div className="mt-6 flex flex-wrap items-center gap-2.5">
              {ctas.map((c, i) => (
                <Button
                  key={i}
                  type="button"
                  variant={c.variant ?? (i === 0 ? 'primary' : 'secondary')}
                  size="sm"
                  data-testid={c.testId}
                  onClick={c.onClick}>
                  {c.icon != null && (
                    <span aria-hidden className="mr-1.5">
                      {c.icon}
                    </span>
                  )}
                  {c.label}
                </Button>
              ))}
            </div>
          )}

          {features != null && features.length > 0 && (
            <section className="mt-9 space-y-3">
              {featuresHeading != null && (
                <p className="text-xs font-semibold uppercase tracking-wide text-content-muted">
                  {featuresHeading}
                </p>
              )}
              <div className="grid gap-3 sm:grid-cols-3">
                {features.map((f, i) => (
                  <div
                    key={i}
                    className="rounded-2xl border border-line bg-surface p-4 shadow-subtle">
                    <div
                      aria-hidden
                      className={`mb-2.5 flex h-9 w-9 items-center justify-center rounded-lg text-lg ${tint}`}>
                      {f.icon}
                    </div>
                    <h3 className="text-sm font-semibold text-content">{f.title}</h3>
                    <p className="mt-1 text-xs leading-relaxed text-content-muted">
                      {f.description}
                    </p>
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
