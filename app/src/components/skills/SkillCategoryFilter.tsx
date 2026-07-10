import ChipTabs from '../layout/ChipTabs';
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
    <ChipTabs<SkillCategory>
      items={categories.map(category => {
        const active = category === selected;
        return {
          id: category,
          label: (
            <span className="flex items-center gap-1.5">
              <span
                className={`inline-flex h-4 w-4 items-center justify-center rounded-full ${
                  active
                    ? skillCategoryChipClassName(category)
                    : 'bg-surface-subtle text-content-muted'
                }`}>
                <SkillCategoryIcon
                  category={category}
                  className={
                    active
                      ? skillCategoryIconClassName(category)
                      : 'text-content-muted'
                  }
                />
              </span>
              {category}
            </span>
          ),
        };
      })}
      value={selected}
      onChange={onChange}
      className="flex gap-2 overflow-x-auto pb-1 scrollbar-hide"
    />
  );
};

export default SkillCategoryFilter;
