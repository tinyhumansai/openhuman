import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import ApiKeysStep from './ApiKeysStep';

describe('Onboarding ApiKeysStep', () => {
  it('renders the OpenAI/Anthropic key form with an "or" divider between OAuth and manual key entry', () => {
    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    expect(screen.getByTestId('onboarding-api-keys-step')).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-api-keys-openai-input')).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-api-keys-anthropic-input')).toBeInTheDocument();

    // The hand-rolled divider divs were replaced with the shared Separator
    // primitive — assert its Radix-driven decorative semantics survive.
    const separators = document.querySelectorAll('[data-slot="separator"]');
    expect(separators).toHaveLength(2);
    separators.forEach(separator => {
      // Decorative (the default): Radix renders `role="none"` rather than
      // `role="separator"`, keeping it out of the a11y tree the same way the
      // hand-rolled `aria-hidden` div it replaced did.
      expect(separator).toHaveAttribute('role', 'none');
    });
  });
});
