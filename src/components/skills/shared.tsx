import { useState } from 'react';

import GoogleIcon from '../../assets/icons/GoogleIcon';
import NotionIcon from '../../assets/icons/notion.svg';
import TelegramIcon from '../../assets/icons/telegram.svg';
import { skillManager } from '../../lib/skills/manager';
import type { SkillConnectionStatus } from '../../lib/skills/types';

// Map skill IDs to icons
export const SKILL_ICONS: Record<string, React.ReactElement> = {
  telegram: <img src={TelegramIcon} alt="Telegram" className="w-5 h-5" />,
  email: <GoogleIcon className="w-5 h-5" />,
  notion: <img src={NotionIcon} alt="Notion" className="w-5 h-5" />,
  github: (
    <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
      <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
    </svg>
  ),
  otter: (
    <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
      <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z" />
    </svg>
  ),
};

// Default icon for unknown skills
export const DefaultIcon = () => (
  <div className="w-5 h-5 rounded-full bg-primary-500/20 flex items-center justify-center">
    <svg className="w-3 h-3 text-primary-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M12 6v6m0 0v6m0-6h6m-6 0H6"
      />
    </svg>
  </div>
);

// Status display text and colors
export const STATUS_DISPLAY: Record<SkillConnectionStatus, { text: string; color: string }> = {
  connected: { text: 'Connected', color: 'text-sage-400' },
  connecting: { text: 'Connecting', color: 'text-amber-400' },
  not_authenticated: { text: 'Not Auth', color: 'text-amber-400' },
  disconnected: { text: 'Disconnected', color: 'text-stone-400' },
  error: { text: 'Error', color: 'text-coral-400' },
  offline: { text: 'Offline', color: 'text-stone-500' },
  setup_required: { text: 'Setup', color: 'text-primary-400' },
};

// Priority order for sorting (lower number = higher priority)
export const STATUS_PRIORITY: Record<SkillConnectionStatus, number> = {
  connected: 1,
  connecting: 2,
  not_authenticated: 3,
  disconnected: 4,
  setup_required: 5,
  offline: 6,
  error: 7,
};

export interface SkillListEntry {
  id: string;
  name: string;
  description: string;
  ignoreInProduction?: boolean;
  icon?: React.ReactElement;
  hasSetup: boolean;
}

// Contextual action button for skills
export function SkillActionButton({
  skill,
  connectionStatus,
  onOpenModal,
}: {
  skill: SkillListEntry;
  connectionStatus: SkillConnectionStatus;
  onOpenModal: () => void;
}) {
  const [loading, setLoading] = useState(false);

  const handleEnable = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setLoading(true);
    try {
      await skillManager.startSkill({
        id: skill.id,
        name: skill.name,
        version: '0.0.0',
        description: skill.description,
        runtime: 'quickjs',
      });
      if (skill.hasSetup) {
        onOpenModal();
      }
    } catch (err) {
      console.error(`Failed to enable ${skill.id}:`, err);
    } finally {
      setLoading(false);
    }
  };

  if (loading) {
    return (
      <div className="px-4 py-1.5 text-xs font-medium text-stone-400 flex-shrink-0 ml-3">
        <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
          <circle
            className="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            strokeWidth="4"
          />
          <path
            className="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
          />
        </svg>
      </div>
    );
  }

  if (connectionStatus === 'offline') {
    return (
      <button
        onClick={handleEnable}
        className="px-4 py-1.5 text-xs font-medium text-sage-300 bg-sage-500/10 border border-sage-500/30 rounded-lg hover:bg-sage-500/20 transition-colors flex-shrink-0 ml-3">
        Enable
      </button>
    );
  }

  if (connectionStatus === 'setup_required') {
    return (
      <button
        onClick={e => {
          e.stopPropagation();
          onOpenModal();
        }}
        className="px-4 py-1.5 text-xs font-medium text-primary-300 bg-primary-500/10 border border-primary-500/30 rounded-lg hover:bg-primary-500/20 transition-colors flex-shrink-0 ml-3">
        Setup
      </button>
    );
  }

  if (connectionStatus === 'error') {
    return (
      <button
        onClick={handleEnable}
        className="px-4 py-1.5 text-xs font-medium text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-lg hover:bg-amber-500/20 transition-colors flex-shrink-0 ml-3">
        Retry
      </button>
    );
  }

  return (
    <button
      onClick={e => {
        e.stopPropagation();
        onOpenModal();
      }}
      className="px-4 py-1.5 text-xs font-medium text-primary-300 bg-primary-500/10 border border-primary-500/30 rounded-lg hover:bg-primary-500/20 transition-colors flex-shrink-0 ml-3">
      Configure
    </button>
  );
}
