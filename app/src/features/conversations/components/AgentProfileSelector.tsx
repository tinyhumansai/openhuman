import { useT } from '../../../lib/i18n/I18nContext';
import type { AgentProfile } from '../../../types/agentProfile';

export function sortAgentProfiles(profiles: AgentProfile[], locale: string): AgentProfile[] {
  return profiles
    .filter(p => !p.builtIn)
    .sort(
      (a, b) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || a.name.localeCompare(b.name, locale)
    );
}

interface AgentProfileSelectorProps {
  profiles: AgentProfile[];
  selectedProfileId: string;
  locale: string;
  onSelect: (profileId: string) => void;
}

export function AgentProfileSelector({
  profiles,
  selectedProfileId,
  locale,
  onSelect,
}: AgentProfileSelectorProps) {
  const { t } = useT();
  return (
    <div
      className="flex h-7 items-center rounded-full border border-line bg-surface-subtle p-0.5"
      role="radiogroup"
      aria-label={t('chat.agentProfile.label')}>
      <button
        type="button"
        role="radio"
        aria-checked={selectedProfileId === 'default'}
        data-analytics-id="chat-header-mode-quick"
        onClick={() => void onSelect('default')}
        className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-all ${
          selectedProfileId === 'default'
            ? 'bg-surface text-content shadow-sm'
            : 'text-content-muted hover:text-content-secondary'
        }`}>
        {t('chat.agentProfile.quick')}
      </button>
      <button
        type="button"
        role="radio"
        aria-checked={selectedProfileId === 'reasoning'}
        data-analytics-id="chat-header-mode-reasoning"
        onClick={() => void onSelect('reasoning')}
        className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-all ${
          selectedProfileId === 'reasoning'
            ? 'bg-surface text-content shadow-sm'
            : 'text-content-muted hover:text-content-secondary'
        }`}>
        {t('chat.agentProfile.reasoning')}
      </button>
      {sortAgentProfiles(profiles, locale).map(profile => (
        <button
          key={profile.id}
          type="button"
          role="radio"
          aria-checked={selectedProfileId === profile.id}
          data-analytics-id={`chat-header-mode-${profile.id}`}
          onClick={() => void onSelect(profile.id)}
          className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-all ${
            selectedProfileId === profile.id
              ? 'bg-surface text-content shadow-sm'
              : 'text-content-muted hover:text-content-secondary'
          }`}>
          {profile.name}
        </button>
      ))}
    </div>
  );
}
