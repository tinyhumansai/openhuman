import type { MascotColor } from './mascotPalette';

const ALL_MASCOT_COLORS: MascotColor[] = ['yellow', 'burgundy', 'black', 'navy', 'green'];

let availableColors: Set<MascotColor> = new Set(['yellow']);
let loadPromise: Promise<void> | null = null;

export function isMascotColorAvailable(color: MascotColor): boolean {
  return availableColors.has(color);
}

export function setAvailableMascotColors(colors: Iterable<MascotColor>): void {
  availableColors = new Set(colors);
  if (!availableColors.has('yellow')) {
    availableColors.add('yellow');
  }
}

interface ManifestVariant {
  color?: string;
}

interface ManifestShape {
  variants?: ManifestVariant[];
}

export function loadMascotManifest(): Promise<void> {
  if (loadPromise) return loadPromise;
  loadPromise = (async () => {
    if (typeof fetch !== 'function') return;
    try {
      const url = new URL('generated/remotion/manifest.json', window.location.href).href;
      const response = await fetch(url, { cache: 'no-cache' });
      if (!response.ok) return;
      const manifest = (await response.json()) as ManifestShape;
      const variants = manifest.variants ?? [];
      const colors = new Set<MascotColor>();
      for (const variant of variants) {
        const color = variant.color as MascotColor | undefined;
        if (color && (ALL_MASCOT_COLORS as string[]).includes(color)) {
          colors.add(color);
        }
      }
      if (colors.size > 0) {
        setAvailableMascotColors(colors);
      }
    } catch {
      // Ignore fetch failures — default availability stays in effect.
    }
  })();
  return loadPromise;
}
