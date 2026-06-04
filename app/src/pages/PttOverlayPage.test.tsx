import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { PttOverlayPage } from './PttOverlayPage';

// Mock @tauri-apps/api/event's listen so we can dispatch fake events.
vi.mock('@tauri-apps/api/event', () => {
  const handlers: Record<string, (e: { payload: unknown }) => void> = {};
  return {
    listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => void) => {
      handlers[name] = handler;
      return () => delete handlers[name];
    }),
    __dispatch: (name: string, payload: unknown) => handlers[name]?.({ payload }),
  };
});

describe('PttOverlayPage', () => {
  it('renders idle state by default', () => {
    render(<PttOverlayPage />);
    expect(screen.getByTestId('ptt-overlay-root')).toHaveAttribute('data-active', 'false');
  });

  it('flips to active when ptt-overlay://active fires with active=true', async () => {
    render(<PttOverlayPage />);
    const evt = await import('@tauri-apps/api/event');
    await act(async () => {
      (evt as unknown as { __dispatch: (n: string, p: unknown) => void }).__dispatch(
        'ptt-overlay://active',
        { active: true, session_id: 1 }
      );
    });
    expect(screen.getByTestId('ptt-overlay-root')).toHaveAttribute('data-active', 'true');
  });
});
