import { Monitor } from 'lucide-react';

import cortexdbLogo from '../../assets/provider-icons/cortexdb.png';
import tinyhumansLogo from '../../assets/provider-icons/tinyhumans.svg';
import { cn } from '../../lib/cn';

export type MemoryProviderOption = 'builtin' | 'apikey' | 'selfhost';

/** TinyHumans' mark, drawn as a CSS mask so it takes the surrounding text colour. */
const TinyHumansMark = ({ className }: { className?: string }) => (
  <span
    aria-hidden
    data-testid="memory-logo-tinyhumans"
    className={cn('inline-block bg-current', className)}
    style={{
      maskImage: `url("${tinyhumansLogo}")`,
      WebkitMaskImage: `url("${tinyhumansLogo}")`,
      maskSize: 'contain',
      WebkitMaskSize: 'contain',
      maskRepeat: 'no-repeat',
      WebkitMaskRepeat: 'no-repeat',
      maskPosition: 'center',
      WebkitMaskPosition: 'center',
    }}
  />
);

/**
 * The brand mark for a memory provider: TinyHumans for the hosted engine and
 * CortexDB's published logo (cortexdb.ai) for the other two, with a small
 * computer badge marking the self-hosted one.
 */
const MemoryProviderLogo = ({
  option,
  className,
}: {
  option: MemoryProviderOption;
  className?: string;
}) => {
  if (option === 'builtin') return <TinyHumansMark className={className} />;
  return (
    <span className="relative inline-flex">
      <img
        src={cortexdbLogo}
        alt=""
        aria-hidden
        data-testid="memory-logo-cortexdb"
        className={cn('object-contain', className)}
      />
      {option === 'selfhost' && (
        <span
          aria-hidden
          className="absolute -right-1.5 -bottom-1.5 flex h-4 w-4 items-center justify-center rounded-full bg-surface text-content ring-1 ring-line">
          <Monitor className="h-2.5 w-2.5" />
        </span>
      )}
    </span>
  );
};

export default MemoryProviderLogo;
