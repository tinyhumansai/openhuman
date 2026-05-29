import type { ReactNode } from 'react';
import type { IconType } from 'react-icons';
import { FaDiscord, FaGlobe, FaTelegramPlane } from 'react-icons/fa';
import { IoChatbubble } from 'react-icons/io5';

import YuanbaoIcon from '../channels/YuanbaoIcon';
import {
  LuBlocks,
  LuBot,
  LuKeyboard,
  LuMessageSquareMore,
  LuMic,
  LuMonitor,
  LuPlugZap,
  LuShare2,
  LuSparkles,
  LuWrench,
} from 'react-icons/lu';

import type { SkillCategory } from './skillCategories';

function iconClasses(...parts: Array<string | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export function SkillIconBadge({
  icon: Icon,
  label,
  bgClassName,
  iconClassName,
  className,
}: {
  icon: IconType;
  label: string;
  bgClassName: string;
  iconClassName: string;
  className?: string;
}) {
  return (
    <span
      role="img"
      aria-label={label}
      className={iconClasses(
        'flex h-8 w-8 items-center justify-center rounded-xl shadow-sm ring-1 ring-black/5',
        bgClassName,
        className
      )}>
      <Icon className={iconClasses('h-[18px] w-[18px]', iconClassName)} aria-hidden="true" />
    </span>
  );
}

export function getChannelIcons(
  t: (key: string, fallback?: string) => string
): Record<string, ReactNode> {
  return {
    telegram: (
    <SkillIconBadge
      icon={FaTelegramPlane}
      label={t('skills.channelIcon.telegram')}
      bgClassName="bg-[#E7F4FB]"
      iconClassName="text-[#249CD8]"
    />
  ),
    discord: (
    <SkillIconBadge
      icon={FaDiscord}
      label={t('skills.channelIcon.discord')}
      bgClassName="bg-[#EEF2FF]"
      iconClassName="text-[#5865F2]"
    />
  ),
    web: (
    <SkillIconBadge
      icon={FaGlobe}
      label={t('skills.channelIcon.web')}
      bgClassName="bg-stone-100 dark:bg-neutral-800"
      iconClassName="text-stone-600 dark:text-neutral-300"
    />
  ),
    imessage: (
    <SkillIconBadge
      icon={IoChatbubble}
      label={t('skills.channelIcon.imessage')}
      bgClassName="bg-[#E8F8EE]"
      iconClassName="text-[#34C759]"
    />
  ),
    yuanbao: (
      <span
        role="img"
        aria-label={t('skills.channelIcon.yuanbao')}
        className="flex h-8 w-8 items-center justify-center rounded-xl shadow-sm ring-1 ring-black/5 bg-white">
        <YuanbaoIcon className="h-[18px] w-[18px]" />
      </span>
    ),
  };
}

const CATEGORY_META: Record<
  SkillCategory,
  { icon: IconType; chipClassName: string; iconClassName: string; headingClassName: string }
> = {
  All: {
    icon: LuBlocks,
    chipClassName: 'bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300',
    iconClassName: 'text-stone-500 dark:text-neutral-400',
    headingClassName: 'text-stone-500 dark:text-neutral-400',
  },
  'Built-in': {
    icon: LuSparkles,
    chipClassName: 'bg-primary-50 text-primary-700',
    iconClassName: 'text-primary-600',
    headingClassName: 'text-primary-600',
  },
  Channels: {
    icon: LuMessageSquareMore,
    chipClassName: 'bg-sky-50 text-sky-700',
    iconClassName: 'text-sky-600',
    headingClassName: 'text-sky-600',
  },
  Productivity: {
    icon: LuBot,
    chipClassName: 'bg-emerald-50 text-emerald-700',
    iconClassName: 'text-emerald-600',
    headingClassName: 'text-emerald-600',
  },
  Chat: {
    icon: LuShare2,
    chipClassName: 'bg-violet-50 text-violet-700',
    iconClassName: 'text-violet-600',
    headingClassName: 'text-violet-600',
  },
  'Tools & Automation': {
    icon: LuWrench,
    chipClassName: 'bg-amber-50 text-amber-700',
    iconClassName: 'text-amber-600',
    headingClassName: 'text-amber-600',
  },
  Social: {
    icon: LuPlugZap,
    chipClassName: 'bg-rose-50 text-rose-700',
    iconClassName: 'text-rose-600',
    headingClassName: 'text-rose-600',
  },
  Platform: {
    icon: LuShare2,
    chipClassName: 'bg-cyan-50 text-cyan-700',
    iconClassName: 'text-cyan-600',
    headingClassName: 'text-cyan-600',
  },
  Other: {
    icon: LuBlocks,
    chipClassName: 'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200',
    iconClassName: 'text-stone-500 dark:text-neutral-400',
    headingClassName: 'text-stone-500 dark:text-neutral-400',
  },
};

export function SkillCategoryIcon({
  category,
  className,
}: {
  category: SkillCategory;
  className?: string;
}) {
  const Icon = CATEGORY_META[category].icon;
  return <Icon className={iconClasses('h-3.5 w-3.5', className)} aria-hidden="true" />;
}

export function skillCategoryChipClassName(category: SkillCategory): string {
  return CATEGORY_META[category].chipClassName;
}

export function skillCategoryIconClassName(category: SkillCategory): string {
  return CATEGORY_META[category].iconClassName;
}

export function skillCategoryHeadingClassName(category: SkillCategory): string {
  return CATEGORY_META[category].headingClassName;
}

export const BUILT_IN_SKILL_ICONS = {
  screenIntelligence: <LuMonitor className="h-5 w-5" aria-hidden="true" />,
  textAutocomplete: <LuKeyboard className="h-5 w-5" aria-hidden="true" />,
  voiceStt: <LuMic className="h-5 w-5" aria-hidden="true" />,
};
