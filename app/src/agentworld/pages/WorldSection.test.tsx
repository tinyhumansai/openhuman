/**
 * Tests for WorldSection — the "Tiny Place" World tab renderer boot UX.
 *
 * Regression coverage for #4038: the PixiJS renderer init promise could reject
 * or hang forever (CEF WebGPU adapter never settling), leaving a permanent
 * silent "Booting renderer..." overlay with no error and no retry. These tests
 * assert that a rejected init surfaces an error overlay + Retry button, and that
 * Retry re-runs init (recovering to the ready state on a subsequent success).
 *
 * The iso engine is mocked at module level — no real WebGL/WebGPU is touched.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import WorldSection from './WorldSection';

// Controllable GameWorld test double. `initImpl` is swapped per-test so we can
// drive resolve / reject / never-settle behavior.
let initImpl: (parent: HTMLElement) => Promise<void>;
const initSpy = vi.fn((parent: HTMLElement) => initImpl(parent));
const setChangeListener = vi.fn();
const setRoom = vi.fn();
const spawnAgents = vi.fn();
const setAutonomous = vi.fn();
const destroy = vi.fn();

vi.mock('../iso', () => {
  class FakeGameWorld {
    public currentRoomKey = 'outside';
    public init = (parent: HTMLElement): Promise<void> => initSpy(parent);
    public setChangeListener = setChangeListener;
    public setRoom = setRoom;
    public spawnAgents = spawnAgents;
    public setAutonomous = setAutonomous;
    public destroy = destroy;
  }
  return {
    GameWorld: FakeGameWorld,
    ROOM_REGISTRY: [
      { key: 'outside', name: 'World', description: 'A large open plaza.' },
      { key: 'poker', name: 'Poker', description: 'Eight seats around a felt table.' },
    ],
    RendererInitError: class RendererInitError extends Error {},
  };
});

beforeEach(() => {
  vi.clearAllMocks();
  initImpl = () => Promise.resolve();
});

describe('WorldSection renderer boot', () => {
  test('shows booting overlay while init is in flight', () => {
    initImpl = () => new Promise<void>(() => {}); // never settles
    render(<WorldSection />);
    expect(screen.getByText(/booting renderer/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });

  test('hides booting overlay and wires the world once init resolves', async () => {
    initImpl = () => Promise.resolve();
    render(<WorldSection />);
    await waitFor(() => {
      expect(screen.queryByText(/booting renderer/i)).not.toBeInTheDocument();
    });
    expect(setRoom).toHaveBeenCalledWith('outside');
    expect(spawnAgents).toHaveBeenCalled();
    expect(setAutonomous).toHaveBeenCalledWith(true);
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });

  test('shows error overlay + Retry when init rejects', async () => {
    initImpl = () => Promise.reject(new Error('webgpu adapter hung'));
    render(<WorldSection />);
    await waitFor(() => {
      expect(screen.getByText(/could not start the world renderer/i)).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
    // The dead-end booting overlay must NOT also be showing.
    expect(screen.queryByText(/booting renderer/i)).not.toBeInTheDocument();
  });

  test('Retry re-invokes init and recovers to ready on success', async () => {
    const user = userEvent.setup();
    // First attempt fails, second succeeds.
    initImpl = () => Promise.reject(new Error('init failed'));
    render(<WorldSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
    });
    expect(initSpy).toHaveBeenCalledTimes(1);

    initImpl = () => Promise.resolve();
    await user.click(screen.getByRole('button', { name: /retry/i }));

    await waitFor(() => {
      expect(initSpy).toHaveBeenCalledTimes(2);
    });
    await waitFor(() => {
      expect(screen.queryByText(/could not start the world renderer/i)).not.toBeInTheDocument();
    });
    expect(setRoom).toHaveBeenCalledWith('outside');
  });
});
