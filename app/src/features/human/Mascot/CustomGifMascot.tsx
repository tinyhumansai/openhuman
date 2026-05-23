import type { MascotFace } from './Ghosty';

export interface CustomGifMascotProps {
  src: string;
  face?: MascotFace;
}

export function CustomGifMascot({ src, face = 'idle' }: CustomGifMascotProps) {
  return (
    <img
      src={src}
      alt=""
      aria-hidden="true"
      data-testid="custom-gif-mascot"
      data-face={face}
      referrerPolicy="no-referrer"
      draggable={false}
      className="block h-full w-full select-none object-contain"
    />
  );
}
