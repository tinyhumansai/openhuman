export type MascotColor = 'yellow' | 'burgundy' | 'black' | 'navy' | 'green';

export interface MascotPalette {
  armHighlightMatrix: string;
  armShadowMatrix: string;
  bodyFill: string;
  bodyHighlightMatrix: string;
  bodyShadowMatrix: string;
  headHighlightMatrix: string;
  headShadowMatrix: string;
  neckShadowColor: string;
}

const YELLOW_PALETTE: MascotPalette = {
  armHighlightMatrix: '0 0 0 0 0.973501 0 0 0 0 0.909066 0 0 0 0 0.671677 0 0 0 1 0',
  armShadowMatrix: '0 0 0 0 0.796078 0 0 0 0 0.576471 0 0 0 0 0.0980392 0 0 0 1 0',
  bodyFill: '#F7D145',
  bodyHighlightMatrix: '0 0 0 0 0.962384 0 0 0 0 0.860378 0 0 0 0 0.484572 0 0 0 1 0',
  bodyShadowMatrix: '0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0',
  headHighlightMatrix: '0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 1 0',
  headShadowMatrix: '0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0',
  neckShadowColor: '#B23C05',
};

const BLACK_PALETTE: MascotPalette = {
  armHighlightMatrix: '0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0',
  armShadowMatrix: '0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 1 0',
  bodyFill: '#3A3A3A',
  bodyHighlightMatrix: '0 0 0 0 0.439078 0 0 0 0 0.439078 0 0 0 0 0.439078 0 0 0 1 0',
  bodyShadowMatrix: '0 0 0 0 0.0229492 0 0 0 0 0.0207891 0 0 0 0 0.0161271 0 0 0 1 0',
  headHighlightMatrix: '0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0',
  headShadowMatrix: '0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 1 0',
  neckShadowColor: '#030100',
};

const palettes: Record<MascotColor, MascotPalette> = {
  yellow: YELLOW_PALETTE,
  burgundy: {
    armHighlightMatrix: '0 0 0 0 0.607843 0 0 0 0 0.235294 0 0 0 0 0.313726 0 0 0 1 0',
    armShadowMatrix: '0 0 0 0 0.27451 0 0 0 0 0.0745098 0 0 0 0 0.129412 0 0 0 1 0',
    bodyFill: '#8A2647',
    bodyHighlightMatrix: '0 0 0 0 0.607843 0 0 0 0 0.235294 0 0 0 0 0.313726 0 0 0 1 0',
    bodyShadowMatrix: '0 0 0 0 0.27451 0 0 0 0 0.0745098 0 0 0 0 0.129412 0 0 0 1 0',
    headHighlightMatrix: '0 0 0 0 0.854902 0 0 0 0 0.611765 0 0 0 0 0.690196 0 0 0 1 0',
    headShadowMatrix: '0 0 0 0 0.27451 0 0 0 0 0.0745098 0 0 0 0 0.129412 0 0 0 1 0',
    neckShadowColor: '#541128',
  },
  black: BLACK_PALETTE,
  navy: {
    armHighlightMatrix: '0 0 0 0 0.270588 0 0 0 0 0.447059 0 0 0 0 0.654902 0 0 0 1 0',
    armShadowMatrix: '0 0 0 0 0.0705882 0 0 0 0 0.14902 0 0 0 0 0.270588 0 0 0 1 0',
    bodyFill: '#234B74',
    bodyHighlightMatrix: '0 0 0 0 0.270588 0 0 0 0 0.447059 0 0 0 0 0.654902 0 0 0 1 0',
    bodyShadowMatrix: '0 0 0 0 0.0705882 0 0 0 0 0.14902 0 0 0 0 0.270588 0 0 0 1 0',
    headHighlightMatrix: '0 0 0 0 0.603922 0 0 0 0 0.760784 0 0 0 0 0.905882 0 0 0 1 0',
    headShadowMatrix: '0 0 0 0 0.0705882 0 0 0 0 0.14902 0 0 0 0 0.270588 0 0 0 1 0',
    neckShadowColor: '#16324D',
  },
  green: {
    armHighlightMatrix: '0 0 0 0 0.403922 0 0 0 0 0.654902 0 0 0 0 0.364706 0 0 0 1 0',
    armShadowMatrix: '0 0 0 0 0.113725 0 0 0 0 0.270588 0 0 0 0 0.117647 0 0 0 1 0',
    bodyFill: '#5FA64F',
    bodyHighlightMatrix: '0 0 0 0 0.403922 0 0 0 0 0.654902 0 0 0 0 0.364706 0 0 0 1 0',
    bodyShadowMatrix: '0 0 0 0 0.113725 0 0 0 0 0.270588 0 0 0 0 0.117647 0 0 0 1 0',
    headHighlightMatrix: '0 0 0 0 0.780392 0 0 0 0 0.894118 0 0 0 0 0.733333 0 0 0 1 0',
    headShadowMatrix: '0 0 0 0 0.113725 0 0 0 0 0.270588 0 0 0 0 0.117647 0 0 0 1 0',
    neckShadowColor: '#2E5A24',
  },
};

export function getMascotPalette(color: MascotColor): MascotPalette {
  return palettes[color];
}
