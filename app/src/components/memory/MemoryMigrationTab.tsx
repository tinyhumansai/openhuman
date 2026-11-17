/**
 * Memory → Migration: everything about bringing memory forward from earlier
 * versions, in one place instead of a banner over every chip. Holds the
 * import-then-organize flow (`MemoryImportBanner`), whose steps stay visible,
 * disabled, once done. The CortexDB announcement lives on Memory → Provider.
 */
import type { ReactNode } from 'react';

import MemoryImportBanner from './MemoryImportBanner';

interface MemoryMigrationTabProps {
  /** Label of the engine imported memory is uploaded to. */
  engineLabel: string;
  /** Shown in place of the import flow while memory is off; it needs an engine. */
  offState?: ReactNode;
}

export default function MemoryMigrationTab({ engineLabel, offState }: MemoryMigrationTabProps) {
  return (
    <div className="w-full space-y-5 animate-fade-up" data-testid="memory-migration-tab">
      {offState ?? <MemoryImportBanner engineLabel={engineLabel} />}
    </div>
  );
}
