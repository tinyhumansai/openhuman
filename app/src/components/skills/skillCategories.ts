export type SkillCategory =
  | 'All'
  | 'Built-in'
  | 'Channels'
  | 'Productivity'
  | 'Chat'
  | 'Tools & Automation'
  | 'Social'
  | 'Platform'
  | 'Other';

export const SKILL_CATEGORY_ORDER: SkillCategory[] = [
  'All',
  'Built-in',
  'Channels',
  'Chat',
  'Productivity',
  'Tools & Automation',
  'Social',
  'Platform',
  'Other',
];

/**
 * Translated label for each category id. The ids themselves (SkillCategory,
 * SKILL_CATEGORY_ORDER) stay English string literals — they are used as map
 * keys and equality checks — only the rendered label goes through t().
 */
export const SKILL_CATEGORY_LABEL_KEYS: Record<SkillCategory, string> = {
  All: 'skills.category.all',
  'Built-in': 'skills.category.builtIn',
  Channels: 'skills.category.channels',
  Productivity: 'skills.category.productivity',
  Chat: 'skills.category.chat',
  'Tools & Automation': 'skills.category.toolsAutomation',
  Social: 'skills.category.social',
  Platform: 'skills.category.platform',
  Other: 'skills.category.other',
};
