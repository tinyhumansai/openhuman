import { describe, expect, it } from 'vitest';

import { selectYellowMascotAsset } from './runtimeAssetCatalog';

describe('selectYellowMascotAsset', () => {
  it('uses default pre-rendered assets for the steady mascot states', () => {
    expect(selectYellowMascotAsset({ face: 'idle', arm: 'none' })).toEqual({
      profile: 'default',
      relativePath: 'generated/remotion/default/yellow/yellow-MascotIdle.png',
      variant: 'yellow-MascotIdle',
    });

    expect(selectYellowMascotAsset({ face: 'speaking', arm: 'none' })).toEqual({
      profile: 'default',
      relativePath: 'generated/remotion/default/yellow/yellow-MascotTalking.png',
      variant: 'yellow-MascotTalking',
    });
  });

  it('maps thinking faces to the thinking asset', () => {
    expect(selectYellowMascotAsset({ face: 'confused', arm: 'none' })).toEqual({
      profile: 'default',
      relativePath: 'generated/remotion/default/yellow/yellow-MascotThinking.png',
      variant: 'yellow-MascotThinking',
    });
  });

  it('selects the requested mascot color family', () => {
    expect(selectYellowMascotAsset({ face: 'idle', arm: 'none', mascotColor: 'navy' })).toEqual({
      profile: 'default',
      relativePath: 'generated/remotion/default/navy/yellow-MascotIdle.png',
      variant: 'yellow-MascotIdle',
    });
  });

  it('uses the compact profile for the bottom-right mascot window styling', () => {
    expect(
      selectYellowMascotAsset({
        face: 'idle',
        arm: 'none',
        groundShadowOpacity: 0.75,
        compactArmShading: true,
      })
    ).toEqual({
      profile: 'compact',
      relativePath: 'generated/remotion/compact/yellow/yellow-MascotIdle.png',
      variant: 'yellow-MascotIdle',
    });
  });

  it('falls back to live remotion for unsupported prop combinations', () => {
    expect(selectYellowMascotAsset({ face: 'idle', arm: 'wave' })).toBeNull();
    expect(
      selectYellowMascotAsset({
        face: 'idle',
        arm: 'none',
        groundShadowOpacity: 0.5,
      })
    ).toBeNull();
  });
});
