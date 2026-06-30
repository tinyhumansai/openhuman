import { useAppSelector } from '../store/hooks';
import { selectEffectiveTheme } from '../store/themeSlice';
import MeshGradient from './MeshGradient';

interface AppBackgroundProps {
  className?: string;
}

/**
 * The app's shared background layer. The backdrop is theme-controlled:
 * - `mesh` (default): animated WebGL mesh gradient (theme-tinted) + dotted canvas.
 * - `solid`: just the dotted canvas over the themed flat/gradient body.
 * - `image`: a cover image over the dotted canvas.
 *
 * Renders as an absolutely-positioned layer that fills its parent; place
 * foreground content in a sibling `relative z-10` container.
 */
export default function AppBackground({ className = '' }: AppBackgroundProps) {
  const theme = useAppSelector(selectEffectiveTheme);
  const backdrop = theme.backdrop?.kind ?? 'mesh';
  const showDots = theme.backdrop?.dots !== false; // default on

  return (
    <div className={`absolute inset-0 overflow-hidden ${className}`} aria-hidden="true">
      {backdrop === 'mesh' && <MeshGradient />}
      {backdrop === 'image' && theme.backdrop?.imageUrl && (
        <div
          className="absolute inset-0 bg-cover bg-center"
          style={{ backgroundImage: `url("${theme.backdrop.imageUrl}")` }}
        />
      )}
      {showDots && <div className="app-dotted-canvas absolute inset-0" />}
    </div>
  );
}
