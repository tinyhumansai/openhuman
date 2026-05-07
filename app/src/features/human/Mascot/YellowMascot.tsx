import createDebug from 'debug';
import { type FC, useEffect, useMemo, useState } from 'react';

import type { MascotFace } from './Ghosty';
import type { MascotColor } from './mascotPalette';
import { selectYellowMascotAsset } from './runtimeAssetCatalog';

const debugMascotAssets = createDebug('openhuman:mascot:assets');

export interface YellowMascotProps {
  face?: MascotFace;
  arm?: 'wave' | 'none';
  size?: number | string;
  groundShadowOpacity?: number;
  compactArmShading?: boolean;
  mascotColor?: MascotColor;
}

export const YellowMascot: FC<YellowMascotProps> = ({
  face = 'idle',
  arm = 'none',
  size = '100%',
  groundShadowOpacity,
  compactArmShading,
  mascotColor = 'yellow',
}) => {
  const [assetFailed, setAssetFailed] = useState(false);
  const asset = useMemo(
    () =>
      selectYellowMascotAsset({
        face,
        arm,
        groundShadowOpacity,
        compactArmShading,
        mascotColor,
      }),
    [face, arm, groundShadowOpacity, compactArmShading, mascotColor]
  );
  const assetUrl = useMemo(
    () => (asset ? new URL(asset.relativePath, window.location.href).href : null),
    [asset]
  );

  useEffect(() => {
    setAssetFailed(false);
  }, [assetUrl]);

  useEffect(() => {
    if (!assetUrl) {
      debugMascotAssets(
        'unsupported-asset-request face=%s arm=%s color=%s shadow=%s compact=%s',
        face,
        arm,
        mascotColor,
        groundShadowOpacity,
        compactArmShading
      );
    }
  }, [arm, assetUrl, compactArmShading, face, groundShadowOpacity, mascotColor]);

  return (
    <div
      style={{
        width: typeof size === 'number' ? `${size}px` : size,
        aspectRatio: '1 / 1',
        background: 'transparent',
      }}
      data-face={face}>
      {assetUrl && !assetFailed ? (
        <img
          key={assetUrl}
          src={assetUrl}
          alt=""
          aria-hidden="true"
          style={{ width: '100%', height: '100%', background: 'transparent' }}
          onError={() => {
            debugMascotAssets('asset-miss face=%s arm=%s color=%s url=%s', face, arm, mascotColor, assetUrl);
            setAssetFailed(true);
          }}
        />
      ) : null}
    </div>
  );
};
