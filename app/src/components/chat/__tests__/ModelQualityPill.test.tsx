import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import ModelQualityPill from '../ModelQualityPill';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('ModelQualityPill', () => {
  it('renders with model name', () => {
    render(<ModelQualityPill />);
    expect(screen.getByText('OpenHuman')).toBeInTheDocument();
  });

  it('renders quality indicator', () => {
    render(<ModelQualityPill />);
    // The quality value comes through t('composer.qualityHigh') which returns the key in test
    expect(screen.getByText('composer.qualityHigh')).toBeInTheDocument();
  });

  it('has chevron icon', () => {
    const { container } = render(<ModelQualityPill />);
    const svg = container.querySelector('svg');
    expect(svg).toBeInTheDocument();
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
