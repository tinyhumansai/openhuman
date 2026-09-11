import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import McpServerCard, { deriveAuthor } from './McpServerCard';
import type { SmitheryServer } from './types';

const server: SmitheryServer = {
  qualified_name: 'acme.corp/example-server',
  display_name: 'Example Server',
  description: 'Does example things.',
};

describe('<McpServerCard />', () => {
  it('renders as a button with the display name and description', () => {
    const { getByRole, getByText } = renderWithProviders(
      <McpServerCard server={server} onSelect={vi.fn()} />
    );
    expect(getByRole('button')).toBeInTheDocument();
    expect(getByText('Example Server')).toBeInTheDocument();
    expect(getByText('Does example things.')).toBeInTheDocument();
  });

  it('calls onSelect with the qualified name when clicked', () => {
    const onSelect = vi.fn();
    const { getByRole } = renderWithProviders(
      <McpServerCard server={server} onSelect={onSelect} />
    );
    getByRole('button').click();
    expect(onSelect).toHaveBeenCalledWith('acme.corp/example-server');
  });
});

describe('deriveAuthor', () => {
  it('derives the last dotted segment before the slash', () => {
    expect(deriveAuthor('acme.corp/example-server')).toBe('corp');
  });

  it('returns null when there is no slash', () => {
    expect(deriveAuthor('example-server')).toBeNull();
  });
});
