import MeshGradient from './MeshGradient';

interface AppBackgroundProps {
  className?: string;
}

/**
 * The app's shared background layer: animated mesh gradient + dotted canvas
 * overlay. Renders as an absolutely-positioned layer that fills its parent,
 * so callers stay in control of layout. Place your foreground content in a
 * sibling `relative z-10` container.
 */
export default function AppBackground({ className = '' }: AppBackgroundProps) {
  return (
    <div className={`absolute inset-0 overflow-hidden ${className}`} aria-hidden="true">
      <MeshGradient />
      <div className="app-dotted-canvas absolute inset-0" />
    </div>
  );
}
