/**
 * Platform selector chip group for the Meetings composer.
 *
 * Renders Google Meet / Zoom / Teams / Webex as keyboard-navigable radio
 * buttons. This is purely a selector: the bot joins via the pasted meeting
 * link regardless of any Composio account, so no connection is required or
 * implied here. (A connected account only ever helps silently auto-fill the
 * display name.)
 */
import debug from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { MeetingPlatform } from '../../services/meetCallService';
import { MEETING_PLATFORMS, platformLabel, platformLogoUrl } from './meetingUtils';

const log = debug('meetings:platform-chips');

export interface PlatformChipsProps {
  selected: MeetingPlatform;
  onSelect: (platform: MeetingPlatform) => void;
  disabled?: boolean;
}

function PlatformLogo({ platform, label }: { platform: MeetingPlatform; label: string }) {
  const [failed, setFailed] = useState(false);

  if (failed) {
    // Fallback: first letter of the platform label
    return (
      <span
        className="flex h-4 w-4 items-center justify-center rounded-sm bg-surface-muted text-[9px] font-bold text-content-muted"
        aria-hidden="true">
        {label.charAt(0)}
      </span>
    );
  }

  return (
    <img
      src={platformLogoUrl(platform)}
      alt=""
      aria-hidden="true"
      className="h-4 w-4 object-contain"
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

export function PlatformChips({ selected, onSelect, disabled = false }: PlatformChipsProps) {
  const { t } = useT();

  function handleClick(platform: MeetingPlatform) {
    if (disabled) return;
    log('[platform-chips] selected platform=%s', platform);
    onSelect(platform);
  }

  function handleKeyDown(event: React.KeyboardEvent, platform: MeetingPlatform) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      handleClick(platform);
    }
  }

  return (
    <div
      role="radiogroup"
      aria-label={t('skills.meetingBots.modalAriaLabel')}
      className="flex flex-wrap gap-2">
      {MEETING_PLATFORMS.map(platform => {
        const isSelected = selected === platform;
        const label = platformLabel(platform, t);

        return (
          <button
            key={platform}
            type="button"
            role="radio"
            aria-checked={isSelected}
            aria-label={label}
            onClick={() => handleClick(platform)}
            onKeyDown={e => handleKeyDown(e, platform)}
            disabled={disabled}
            className={[
              'flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-medium',
              'transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2',
              'focus-visible:ring-primary-500 focus-visible:ring-offset-1',
              'disabled:cursor-not-allowed disabled:opacity-40',
              isSelected
                ? 'border-primary-500 bg-primary-50 text-primary-700 dark:bg-primary-500/15 dark:text-primary-300'
                : 'border-line bg-surface text-content-secondary hover:border-primary-300 hover:bg-primary-50/40 dark:hover:bg-primary-500/10',
            ].join(' ')}>
            <PlatformLogo platform={platform} label={label} />
            <span>{label}</span>
          </button>
        );
      })}
    </div>
  );
}
