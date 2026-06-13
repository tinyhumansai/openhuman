import type { ToastNotification } from '../../types/intelligence';
import { MemoryWorkspace } from './MemoryWorkspace';

/**
 * Memory tab body.
 *
 * Renders the core memory experience (`MemoryWorkspace` — sources registry,
 * tree-status panel, and summary graph) directly, with no sub-pill/tab bar.
 *
 * The internal graph/memory-analysis surfaces (Diagram, Centrality, Cohesion,
 * Associations, Freshness, Timeline, Paths, Namespaces) are developer/analysis
 * views that cluttered the experience and are intentionally no longer reachable
 * from this tab. Their components are retained on disk (and still unit-tested)
 * so they can be restored behind a dedicated surface later if needed.
 */
interface MemorySectionProps {
  onToast: (toast: Omit<ToastNotification, 'id'>) => void;
}

export default function MemorySection({ onToast }: MemorySectionProps) {
  return <MemoryWorkspace onToast={onToast} />;
}
