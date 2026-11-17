import cogneeLogo from '../../assets/provider-icons/cognee.png';
import lettaLogo from '../../assets/provider-icons/letta.png';
import mem0Logo from '../../assets/provider-icons/mem0.svg';
import supermemoryLogo from '../../assets/provider-icons/supermemory.svg';
import zepLogo from '../../assets/provider-icons/zep.png';
import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { Badge } from '../ui';

/**
 * Memory engines OpenHuman does not run yet, shown greyed out so people can
 * see what is coming. Logos are each project's published mark, bundled
 * locally. `mask` marks a single-colour logo drawn in the text colour.
 */
const UPCOMING = [
  { id: 'supermemory', name: 'Supermemory', logo: supermemoryLogo, mask: true },
  { id: 'mem0', name: 'Mem0', logo: mem0Logo, mask: false },
  { id: 'cognee', name: 'Cognee', logo: cogneeLogo, mask: false },
  { id: 'zep', name: 'Zep', logo: zepLogo, mask: false },
  { id: 'letta', name: 'Letta', logo: lettaLogo, mask: false },
] as const;

/** Description key per engine (explicit so the i18n audit sees them). */
const DESC_KEY: Record<(typeof UPCOMING)[number]['id'], string> = {
  supermemory: 'memoryPage.engine.soon.supermemory',
  mem0: 'memoryPage.engine.soon.mem0',
  cognee: 'memoryPage.engine.soon.cognee',
  zep: 'memoryPage.engine.soon.zep',
  letta: 'memoryPage.engine.soon.letta',
};

export default function MemoryComingSoon() {
  const { t } = useT();
  return (
    <section className="flex flex-col gap-2.5" data-testid="memory-engines-soon">
      <header>
        <h3 className="text-sm font-semibold text-content">{t('memoryPage.engine.soon.title')}</h3>
        <p className="text-xs text-content-muted">{t('memoryPage.engine.soon.description')}</p>
      </header>
      <div className="grid gap-2.5 @md:grid-cols-2 @3xl:grid-cols-3">
        {UPCOMING.map(engine => (
          <div
            key={engine.id}
            aria-disabled="true"
            data-testid={`memory-engine-soon-${engine.id}`}
            className="flex items-start gap-3 rounded-xl border border-dashed border-line bg-surface px-3.5 py-3">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-surface-strong opacity-70 grayscale">
              {engine.mask ? (
                <span
                  aria-hidden
                  className="inline-block h-5 w-5 bg-current text-content"
                  style={{
                    maskImage: `url("${engine.logo}")`,
                    WebkitMaskImage: `url("${engine.logo}")`,
                    maskSize: 'contain',
                    WebkitMaskSize: 'contain',
                    maskRepeat: 'no-repeat',
                    WebkitMaskRepeat: 'no-repeat',
                    maskPosition: 'center',
                    WebkitMaskPosition: 'center',
                  }}
                />
              ) : (
                <img
                  src={engine.logo}
                  alt=""
                  aria-hidden
                  className={cn('h-6 w-6 rounded-md object-contain')}
                />
              )}
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-medium text-content-secondary">{engine.name}</span>
                <Badge>{t('memoryPage.engine.soon.badge')}</Badge>
              </div>
              <p className="line-clamp-2 text-xs text-content-muted">{t(DESC_KEY[engine.id])}</p>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
