import { cn } from '../../lib/cn';

export interface EmptyStateProps {
  label: string;
  className?: string;
  /**
   * `data-testid` for the rendered element.
   *
   * Declared and forwarded rather than left to fall through: TypeScript allows
   * ANY hyphenated attribute on a JSX element without checking it against the
   * props type, so a caller passing `data-testid` to a component that does not
   * forward it gets no error, no warning, and no attribute — the id simply is
   * not there, and the spec looking for it fails somewhere unrelated. Every
   * other primitive that specs target (`ListRow`, `ModalShell`) already takes
   * one.
   */
  'data-testid'?: string;
}

const EmptyState = ({ label, className, 'data-testid': testId }: EmptyStateProps) => (
  <p
    data-slot="empty-state"
    data-testid={testId}
    className={cn('px-4 py-4 text-xs italic text-content-faint', className)}>
    {label}
  </p>
);

export default EmptyState;
