import { useAppSelector } from '../store/hooks';
import { selectEffectiveTheme } from '../store/themeSlice';
import MeshGradient from './MeshGradient';

interface AppBackgroundProps {
  className?: string;
}

/**
 * The app's shared background layer. The backdrop is theme-controlled:
 * - `solid` (default): the themed flat body, no gradient. The default is flat
 *   on purpose; the animated shader is opt-in.
 * - `mesh`: animated WebGL mesh gradient (theme-tinted).
 * - `image`: a cover image.
 *
 * A dotted-canvas overlay used to sit above all three, with a Theme Studio
 * toggle and a `backdrop.dots` field behind it. It was removed outright rather
 * than defaulted off, so there is no setting to rediscover and no config key
 * left to honour — a persisted custom theme carrying `dots` simply has an
 * extra property nothing reads.
 *
 * Renders as an absolutely-positioned layer that fills its parent; place
 * foreground content in a sibling `relative z-10` container.
 */
export default function AppBackground({ className = '' }: AppBackgroundProps) {
  const theme = useAppSelector(selectEffectiveTheme);
  const backdrop = theme.backdrop?.kind ?? 'solid';

  return (
    <div className={`absolute inset-0 overflow-hidden ${className}`} aria-hidden="true">
      {backdrop === 'mesh' && <MeshGradient />}
      {backdrop === 'image' && theme.backdrop?.imageUrl && (
        <div
          className="absolute inset-0 bg-cover bg-center"
          style={{ backgroundImage: `url("${theme.backdrop.imageUrl}")` }}
        />
      )}
    </div>
  );
}
