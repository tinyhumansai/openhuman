import { useState } from 'react';

import WhatLeavesMyComputerSheet from './WhatLeavesMyComputerSheet';

export interface WhatLeavesLinkProps {
  label?: string;
  className?: string;
}

/**
 * Inline "what leaves my computer?" trigger. Place near any screen that may
 * cause a network call (model download, skill connect, provider selection).
 * Invisible when not needed, one click away when it is.
 */
const WhatLeavesLink = ({ label = 'What leaves my computer?', className }: WhatLeavesLinkProps) => {
  const [open, setOpen] = useState(false);
  const base =
    'text-sm text-neutral-500 underline underline-offset-2 hover:text-neutral-700 ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/25 ' +
    'focus-visible:ring-offset-2 rounded-sm';
  return (
    <>
      <button
        type="button"
        className={[base, className ?? ''].filter(Boolean).join(' ')}
        onClick={() => setOpen(true)}>
        {label}
      </button>
      <WhatLeavesMyComputerSheet open={open} onClose={() => setOpen(false)} />
    </>
  );
};

export default WhatLeavesLink;
