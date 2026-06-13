import PillTabBar from '../PillTabBar';
import type { SkillCategory } from './skillCategories';
import {
  skillCategoryChipClassName,
  SkillCategoryIcon,
  skillCategoryIconClassName,
} from './skillIcons';

interface SkillCategoryFilterProps {
  categories: SkillCategory[];
  selected: SkillCategory;
  onChange: (category: SkillCategory) => void;
}

const SkillCategoryFilter = ({ categories, selected, onChange }: SkillCategoryFilterProps) => {
  return (
    <PillTabBar
      items={categories.map(category => ({ label: category, value: category }))}
      selected={selected}
      onChange={onChange}
      renderItem={(item, active) => (
        <span className="flex items-center gap-1.5">
          <span
            className={`inline-flex h-4 w-4 items-center justify-center rounded-full ${
              active ? skillCategoryChipClassName(item.value) : 'bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400'
            }`}>
            <SkillCategoryIcon
              category={item.value}
              className={active ? skillCategoryIconClassName(item.value) : 'text-stone-500 dark:text-neutral-400'}
            />
          </span>
          {item.label}
        </span>
      )}
    />
  );
};

export default SkillCategoryFilter;
