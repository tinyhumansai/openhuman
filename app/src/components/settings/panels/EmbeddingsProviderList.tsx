/**
 * Provider radio list for the Embeddings panel. Extracted out of
 * `EmbeddingsPanel.tsx` to keep that file under the repo's file-size
 * convention.
 */
import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import type { EmbeddingProviderEntry } from '../../../services/api/embeddingsApi';
import { Button } from '../../ui';
import { SettingsBadge, SettingsSection } from '../controls';

export interface EmbeddingsProviderListProps {
  providers: readonly EmbeddingProviderEntry[];
  selectedProvider: string;
  isLocalSession: boolean;
  onSelect: (entry: EmbeddingProviderEntry) => void;
}

const EmbeddingsProviderList = ({
  providers,
  selectedProvider,
  isLocalSession,
  onSelect,
}: EmbeddingsProviderListProps) => {
  const { t } = useT();

  return (
    <SettingsSection>
      <div role="radiogroup" aria-label={t('settings.embeddings.providerAria')}>
        {providers.map((entry, idx) => {
          const selected = entry.slug === selectedProvider;
          return (
            <Button
              key={entry.slug}
              variant="tertiary"
              role="radio"
              aria-checked={selected}
              onClick={() => onSelect(entry)}
              className={cn(
                'h-auto w-full items-start justify-start gap-3 rounded-none px-4 py-3 text-left font-normal',
                idx !== 0 && 'border-t border-line-subtle',
                selected && 'bg-primary-50 dark:bg-primary-500/10'
              )}>
              <span className="flex-1 min-w-0">
                <span className="flex items-center gap-2">
                  <span className="text-sm font-medium text-content">{entry.label}</span>
                  {entry.requires_api_key && (
                    <SettingsBadge variant={entry.has_api_key ? 'success' : 'warning'}>
                      {entry.has_api_key
                        ? t('settings.embeddings.statusConfigured')
                        : t('settings.embeddings.statusNeedsKey')}
                    </SettingsBadge>
                  )}
                  {isLocalSession && entry.slug === 'managed' && (
                    <SettingsBadge variant="warning">
                      {t('settings.embeddings.requiresSignIn')}
                    </SettingsBadge>
                  )}
                </span>
                <span className="block mt-0.5 text-xs text-content-muted">{entry.description}</span>
              </span>
              {selected && (
                <svg
                  className="w-5 h-5 text-primary-500 shrink-0 mt-0.5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  aria-hidden>
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M5 13l4 4L19 7"
                  />
                </svg>
              )}
            </Button>
          );
        })}
      </div>
    </SettingsSection>
  );
};

export default EmbeddingsProviderList;
