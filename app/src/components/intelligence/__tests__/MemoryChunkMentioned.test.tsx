import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { EntityRef } from '../../../utils/tauriCommands';
import { MemoryChunkMentioned } from '../MemoryChunkMentioned';

const ENTITIES: EntityRef[] = [
  { entity_id: 'person:steve', kind: 'person', surface: 'Steven Enamakel', count: 4 },
  { entity_id: 'org:tinyhumans', kind: 'organization', surface: 'TinyHumans', count: 1 },
  { entity_id: 'event:launch', kind: 'event', surface: 'Phoenix launch', count: 7 },
];

describe('MemoryChunkMentioned', () => {
  it('renders nothing when the entity list is empty', () => {
    const { container } = render(<MemoryChunkMentioned entities={[]} onSelectEntity={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders one row per entity with kind, surface, and a pluralised count', () => {
    render(<MemoryChunkMentioned entities={ENTITIES} onSelectEntity={vi.fn()} />);
    expect(screen.getByText('Steven Enamakel')).toBeInTheDocument();
    expect(screen.getByText('TinyHumans')).toBeInTheDocument();
    expect(screen.getByText('Phoenix launch')).toBeInTheDocument();

    // Singular vs plural — the surface display has to switch on count.
    expect(screen.getByText('1 chunk')).toBeInTheDocument();
    expect(screen.getByText('4 chunks')).toBeInTheDocument();
    expect(screen.getByText('7 chunks')).toBeInTheDocument();
  });

  it('fires onSelectEntity with the clicked entity', () => {
    const onSelectEntity = vi.fn();
    render(<MemoryChunkMentioned entities={ENTITIES} onSelectEntity={onSelectEntity} />);

    fireEvent.click(screen.getByText('TinyHumans').closest('button')!);
    expect(onSelectEntity).toHaveBeenCalledTimes(1);
    expect(onSelectEntity).toHaveBeenCalledWith(ENTITIES[1]);
  });
});
