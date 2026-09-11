import type { ReactNode } from 'react';
import type { IconType } from 'react-icons';
import { FaDiscord, FaGlobe, FaTelegramPlane } from 'react-icons/fa';
import { IoChatbubble } from 'react-icons/io5';
import {
  LuBlocks,
  LuBot,
  LuMessageSquareMore,
  LuPlugZap,
  LuShare2,
  LuSparkles,
  LuWrench,
} from 'react-icons/lu';

import YuanbaoIcon from '../channels/YuanbaoIcon';
import { cn } from '../../lib/cn';
import type { SkillCategory } from './skillCategories';

function SkillIconBadge({
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
      className={cn(
        'flex h-8 w-8 items-center justify-center rounded-xl shadow-xs ring-1 ring-surface-overlay/5',
        bgClassName,
        className
      )}>
      <Icon className={cn('h-[18px] w-[18px]', iconClassName)} aria-hidden="true" />
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
        bgClassName="bg-surface-subtle"
        iconClassName="text-content-secondary"
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
        className="flex h-8 w-8 items-center justify-center rounded-xl shadow-xs ring-1 ring-surface-overlay/5 bg-surface">
        <YuanbaoIcon className="h-[18px] w-[18px]" />
      </span>
    ),
  };
}

/**
 * Category tone table. The app has four themeable ramps (primary / sage /
 * amber / coral) and nine categories, so only the four categories whose
 * identity a reader acts on keep a hue — the shipped tier (`Built-in`), the
 * two behavioural families (`Productivity`, `Social`) and the one that already
 * carries a caution tone (`Tools & Automation`). The surplus rows fall back to
 * the neutral pair defined by `All` / `Other` rather than reaching for a fifth,
 * unthemeable ramp. See `gitbooks/developing/theming.md` ("Colour as identity").
 */
const NEUTRAL_CATEGORY_TONE = {
  chipClassName: 'bg-surface-subtle text-content-secondary',
  iconClassName: 'text-content-muted',
  headingClassName: 'text-content-muted',
} as const;

const CATEGORY_META: Record<
  SkillCategory,
  { icon: IconType; chipClassName: string; iconClassName: string; headingClassName: string }
> = {
  All: {
    icon: LuBlocks,
    ...NEUTRAL_CATEGORY_TONE,
  },
  'Built-in': {
    icon: LuSparkles,
    chipClassName: 'bg-primary-50 text-primary-700',
    iconClassName: 'text-primary-600',
    headingClassName: 'text-primary-600',
  },
  Channels: {
    icon: LuMessageSquareMore,
    ...NEUTRAL_CATEGORY_TONE,
  },
  Productivity: {
    icon: LuBot,
    chipClassName: 'bg-sage-50 text-sage-700',
    iconClassName: 'text-sage-600',
    headingClassName: 'text-sage-600',
  },
  Chat: {
    icon: LuShare2,
    ...NEUTRAL_CATEGORY_TONE,
  },
  'Tools & Automation': {
    icon: LuWrench,
    chipClassName: 'bg-amber-50 text-amber-700',
    iconClassName: 'text-amber-600',
    headingClassName: 'text-amber-600',
  },
  Social: {
    icon: LuPlugZap,
    chipClassName: 'bg-coral-50 text-coral-700',
    iconClassName: 'text-coral-600',
    headingClassName: 'text-coral-600',
  },
  Platform: {
    icon: LuShare2,
    ...NEUTRAL_CATEGORY_TONE,
  },
  Other: {
    icon: LuBlocks,
    ...NEUTRAL_CATEGORY_TONE,
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
  return <Icon className={cn('h-3.5 w-3.5', className)} aria-hidden="true" />;
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
