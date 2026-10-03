import { Navigate, useLocation } from 'react-router-dom';

import { resolveMemoryChip } from '../memory/memoryChips';

/**
 * Back-compat redirect for the retired top-level `/brain` page, which now lives
 * at Connections → Integrations → Memory (`/connections?tab=brain`).
 *
 * Brain used to own `?tab=` (graph|goals|sources|sync|welcome) and `?view=`.
 * Connections owns `?tab=` now, so the old sub-tab moves to `?brain=`, mapped
 * to its Memory v2 chip (graph|goals → ask, sources|sync|history → documents).
 * A value that names no chip is dropped so the page picks its own default.
 * v1's `view` param has no meaning any more and is dropped too; every other
 * param and the hash are carried over unchanged.
 */
export default function BrainRedirect() {
  const { search, hash } = useLocation();
  const legacy = new URLSearchParams(search);
  const params = new URLSearchParams({ tab: 'brain' });
  const legacyTab = legacy.get('tab');
  const view = legacy.get('view');
  const chip = resolveMemoryChip(view === 'history' ? 'history' : legacyTab);
  if (chip) params.set('brain', chip);
  legacy.forEach((value, key) => {
    if (key !== 'tab' && key !== 'view') params.append(key, value);
  });
  return <Navigate to={`/connections?${params.toString()}${hash}`} replace />;
}
