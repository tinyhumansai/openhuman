import { cleanup } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { MascotFrameProducer, sampleCanvasPixels } from '../MascotFrameProducer';

// @tauri-apps/api/event is already mocked in setup.ts (listen → vi.fn())

describe('MascotFrameProducer', () => {
  afterEach(() => cleanup());

  it('renders nothing when no bus session is active', () => {
    const { container } = renderWithProviders(<MascotFrameProducer />);
    // Component returns null until a meet-video:bus-started Tauri event fires
    expect(container.firstChild).toBeNull();
  });

  it('mounts and unmounts without throwing', () => {
    expect(() => {
      const { unmount } = renderWithProviders(<MascotFrameProducer />);
      unmount();
    }).not.toThrow();
  });
});

describe('sampleCanvasPixels', () => {
  it('returns pixel stats for a canvas with mid-range luma', () => {
    // luma = 0.299*128 + 0.587*128 + 0.114*128 ≈ 128
    const mockCtx = {
      getImageData: vi.fn().mockReturnValue({ data: [128, 128, 128, 255] }),
    } as unknown as OffscreenCanvasRenderingContext2D;

    const result = sampleCanvasPixels(mockCtx, 320, 240);
    expect(result).toMatchObject({
      avgLuma: 128,
      minLuma: 128,
      maxLuma: 128,
      darkSamples: 0,
      brightSamples: 35, // all 35 samples have luma > 32
      sampleCount: 35, // 7 cols × 5 rows
    });
  });

  it('counts dark samples correctly for near-black pixels', () => {
    // luma ≈ 0.299*4 + 0.587*4 + 0.114*4 ≈ 4 → dark (< 8), not bright (> 32)
    const mockCtx = {
      getImageData: vi.fn().mockReturnValue({ data: [4, 4, 4, 255] }),
    } as unknown as OffscreenCanvasRenderingContext2D;

    const result = sampleCanvasPixels(mockCtx, 320, 240);
    expect(result).toMatchObject({ darkSamples: 35, brightSamples: 0 });
  });

  it('returns an error object when getImageData throws', () => {
    const mockCtx = {
      getImageData: vi.fn().mockImplementation(() => {
        throw new Error('canvas tainted');
      }),
    } as unknown as OffscreenCanvasRenderingContext2D;

    const result = sampleCanvasPixels(mockCtx, 320, 240);
    expect(result).toMatchObject({ error: 'canvas tainted' });
  });
});
