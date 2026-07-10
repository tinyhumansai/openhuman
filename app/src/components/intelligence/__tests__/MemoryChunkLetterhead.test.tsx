import { render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Chunk } from '../../../utils/tauriCommands';
import { MemoryChunkLetterhead } from '../MemoryChunkLetterhead';

const BASE_CHUNK: Chunk = {
  id: 'chunk-letterhead-01',
  source_kind: 'email',
  source_id: 'gmail:steve@example.com|sanil@vezures.xyz',
  source_ref: 'gmail://msg/abc',
  owner: 'sanil@vezures.xyz',
  timestamp_ms: Date.UTC(2026, 4, 4, 9, 14, 0),
  token_count: 100,
  lifecycle_status: 'admitted',
  content_preview: 'hello',
  has_embedding: true,
  tags: [],
};

describe('MemoryChunkLetterhead', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders the from/to/date frontmatter from a personalized email source', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-05-04T14:30:00'));

    const chunk: Chunk = {
      ...BASE_CHUNK,
      timestamp_ms: Date.now(),
      tags: ['person/Steven-Enamakel'],
    };
    render(<MemoryChunkLetterhead chunk={chunk} />);

    expect(screen.getByText('from')).toBeInTheDocument();
    expect(screen.getByText('to')).toBeInTheDocument();
    // Person tag wins over the raw email handle as the display name.
    expect(screen.getByText('Steven Enamakel')).toBeInTheDocument();
    // The raw address is rendered as secondary text.
    expect(screen.getByText('steve@example.com')).toBeInTheDocument();
    expect(screen.getByText('sanil@vezures.xyz')).toBeInTheDocument();
    expect(screen.getByText('2026·05·04 · 14:30')).toBeInTheDocument();
  });

  it('formats the date using local time instead of UTC components', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-05-04T14:44:00'));

    const chunk: Chunk = { ...BASE_CHUNK, timestamp_ms: Date.now() };
    render(<MemoryChunkLetterhead chunk={chunk} />);

    expect(screen.getByText('2026·05·04 · 14:44')).toBeInTheDocument();
  });

  it('trims whitespace around source sender and recipient values', () => {
    const chunk: Chunk = {
      ...BASE_CHUNK,
      source_id: ' gmail:steve@example.com | sanil@vezures.xyz ',
    };
    render(<MemoryChunkLetterhead chunk={chunk} />);

    expect(screen.getByText('steve@example.com')).toBeInTheDocument();
    expect(screen.getByText('sanil@vezures.xyz')).toBeInTheDocument();
  });

  it('normalizes underscore-separated person tags for display', () => {
    const chunk: Chunk = { ...BASE_CHUNK, tags: ['person/Steven_Enamakel'] };
    render(<MemoryChunkLetterhead chunk={chunk} />);

    expect(screen.getByText('Steven Enamakel')).toBeInTheDocument();
  });

  it('falls back to the raw email when no person/* tag is present', () => {
    render(<MemoryChunkLetterhead chunk={BASE_CHUNK} />);
    // Without a person tag, fromName === the raw email.
    expect(screen.getByText('steve@example.com')).toBeInTheDocument();
  });

  it('uses the owner when the parsed recipient is blank', () => {
    const chunk: Chunk = {
      ...BASE_CHUNK,
      source_id: 'gmail:steve@example.com|   ',
      owner: 'fallback@example.com',
    };
    render(<MemoryChunkLetterhead chunk={chunk} />);

    expect(screen.getByText('fallback@example.com')).toBeInTheDocument();
  });

  it('falls back to the chunk owner when the source_id has no recipient half', () => {
    const chunk: Chunk = {
      ...BASE_CHUNK,
      source_id: 'notion:launch-plan',
      owner: 'sanil@vezures.xyz',
    };
    render(<MemoryChunkLetterhead chunk={chunk} />);
    // No `|` → recipient defaults to owner.
    expect(screen.getByText('sanil@vezures.xyz')).toBeInTheDocument();
  });

  it('uses source_kind as the display when source_id is bare', () => {
    const chunk: Chunk = { ...BASE_CHUNK, source_kind: 'doc', source_id: '', tags: [] };
    render(<MemoryChunkLetterhead chunk={chunk} />);
    // Empty source_id → fromName falls back to the source_kind label.
    expect(screen.getByText('doc')).toBeInTheDocument();
  });
});
