/**
 * Renders the whole primitive layer at once, via the `/dev/ui` gallery, and
 * runs axe over it.
 *
 * The gallery already exists as a visual review surface, so pointing a test at
 * it costs one file and covers every primitive in one pass — including the
 * combinations a per-component test would not think to build (a disabled row, a
 * danger icon-only button, an indeterminate checkbox).
 *
 * jsdom cannot compute layout, so axe auto-skips colour-contrast; what this
 * catches is the structural half — unlabelled controls, invalid ARIA, orphaned
 * `for`/`id` pairs. That is precisely the class of bug the hand-rolled
 * toggles-as-buttons and scrimless overlays were shipping.
 */
import { render } from '@testing-library/react';
import { axe } from 'jest-axe';
import { describe, expect, it, vi } from 'vitest';

import UiGallery from '../../../pages/dev/UiGallery';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('UI primitives', () => {
  it('renders every primitive without crashing', () => {
    const { getByRole } = render(<UiGallery />);
    expect(getByRole('heading', { level: 1, name: 'UI primitives' })).toBeInTheDocument();
  });

  it('has no axe violations across the primitive layer', async () => {
    const { container } = render(<UiGallery />);
    const results = await axe(container);
    expect(results.violations).toEqual([]);
  });
});
