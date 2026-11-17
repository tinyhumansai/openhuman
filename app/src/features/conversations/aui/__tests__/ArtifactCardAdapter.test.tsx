import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { ArtifactSnapshot } from '../../../../store/chatRuntimeSlice';
import { ArtifactCardAdapter, SAVED_NOT_SHOWN } from '../ArtifactCardAdapter';

describe('ArtifactCardAdapter', () => {
  it('renders the generating (in_progress) state', () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-1',
      kind: 'document',
      title: 'Report',
      status: 'in_progress',
      updatedAt: 0,
    };
    render(<ArtifactCardAdapter artifact={artifact} />);
    expect(screen.getByText('Report')).toBeInTheDocument();
  });

  it('renders the ready state with a formatted size', () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-1',
      kind: 'presentation',
      title: 'Quarterly Deck',
      status: 'ready',
      sizeBytes: 4096,
      path: 'a-1/deck.pptx',
      updatedAt: 0,
    };
    render(<ArtifactCardAdapter artifact={artifact} />);
    expect(screen.getByText('Quarterly Deck')).toBeInTheDocument();
  });

  it('renders a Retry action for a failed artifact and calls onRetry with its id', async () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-2',
      kind: 'document',
      title: 'Report',
      status: 'failed',
      error: 'producer crashed',
      updatedAt: 0,
    };
    const onRetry = vi.fn();
    render(<ArtifactCardAdapter artifact={artifact} onRetry={onRetry} />);
    const retry = screen.getByRole('button');
    await userEvent.click(retry);
    expect(onRetry).toHaveBeenCalledWith('a-2');
  });

  it('renders no Retry action for a failed artifact when onRetry is omitted', () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-2',
      kind: 'document',
      title: 'Report',
      status: 'failed',
      error: 'producer crashed',
      updatedAt: 0,
    };
    render(<ArtifactCardAdapter artifact={artifact} />);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('a saved version the card could not show says so, opens, and offers no Retry', async () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-3',
      kind: 'document',
      title: 'Report',
      status: 'failed',
      error: `${SAVED_NOT_SHOWN} version 2 is saved, but the file could not be written`,
      updatedAt: 0,
    };
    const onRetry = vi.fn();
    const onOpenPreview = vi.fn();
    render(
      <ArtifactCardAdapter artifact={artifact} onRetry={onRetry} onOpenPreview={onOpenPreview} />
    );
    expect(screen.getByText(/Saved, but not updated in your Files folder/)).toBeTruthy();
    expect(screen.queryByText('Retry')).toBeNull();
    // Its one button opens the preview, which reads the saved version.
    await userEvent.click(screen.getByRole('button'));
    expect(onOpenPreview).toHaveBeenCalledWith('a-3');
    expect(onRetry).not.toHaveBeenCalled();
  });
});
