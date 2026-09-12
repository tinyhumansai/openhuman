import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import ModelQualityPill from '../ModelQualityPill';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('ModelQualityPill', () => {
  it('renders with model name', () => {
    render(<ModelQualityPill />);
    expect(screen.getByText('OpenHuman')).toBeInTheDocument();
  });

  /**
   * Managed passthrough ids are encoded bare (no `providerSlug:` prefix). The
   * label must not split them on `:` — doing so rendered
   * `openrouter/nex-agi/nex-n2.5-mini:free` as just "free".
   */
  it('labels a managed passthrough model without stripping at the colon', () => {
    render(<ModelQualityPill value="openrouter/nex-agi/nex-n2.5-mini:free" />);
    expect(screen.getByText('nex-n2.5-mini:free')).toBeInTheDocument();
  });

  it('labels a managed passthrough model with no variant tag', () => {
    render(<ModelQualityPill value="openrouter/deepseek/deepseek-v4-flash" />);
    expect(screen.getByText('deepseek-v4-flash')).toBeInTheDocument();
  });

  /** A BYOK value still shows just the model, as before. */
  it('still strips the provider prefix from a BYOK value', () => {
    render(<ModelQualityPill value="openai:gpt-4o-mini" />);
    expect(screen.getByText('gpt-4o-mini')).toBeInTheDocument();
  });

  it('has chevron icon', () => {
    const { container } = render(<ModelQualityPill />);
    const svg = container.querySelector('svg');
    expect(svg).toBeInTheDocument();
  });

  it('gives the pill trailing padding so the chevron is not clipped (#3292)', () => {
    render(<ModelQualityPill />);
    const button = screen.getByRole('button', { name: 'composer.modelSelector' });
    // Horizontal padding + rounded shape keep the trailing chevron fully
    // inside the pill instead of flush against its right edge.
    expect(button).toHaveClass('px-2');
    expect(button).toHaveClass('rounded-md');
    // The chevron itself must not shrink/clip when space is tight.
    const svg = button.querySelector('svg');
    expect(svg).toHaveClass('shrink-0');
  });

  it('has model selector aria-label', () => {
    render(<ModelQualityPill />);
    expect(screen.getByRole('button', { name: 'composer.modelSelector' })).toBeInTheDocument();
  });

  it('applies optional className', () => {
    const { container } = render(<ModelQualityPill className="my-custom-class" />);
    expect(container.firstChild).toHaveClass('my-custom-class');
  });
});
