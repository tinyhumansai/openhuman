import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryMigrationTab from './MemoryMigrationTab';

vi.mock('./MemoryImportBanner', () => ({
  default: ({ engineLabel }: { engineLabel: string }) => (
    <div data-testid="stub-import">{engineLabel}</div>
  ),
}));

describe('MemoryMigrationTab', () => {
  it('shows the import flow', () => {
    renderWithProviders(<MemoryMigrationTab engineLabel="TinyHumans" />);
    expect(screen.getByTestId('stub-import')).toHaveTextContent('TinyHumans');
  });

  it('shows the off state in place of the import flow', () => {
    renderWithProviders(
      <MemoryMigrationTab engineLabel="TinyHumans" offState={<div data-testid="off" />} />
    );
    expect(screen.getByTestId('off')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-import')).not.toBeInTheDocument();
  });
});
