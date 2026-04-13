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
    <div className="flex gap-2 overflow-x-auto pb-1 scrollbar-hide">
      {categories.map(cat => (
        <button
          key={cat}
          type="button"
          aria-pressed={selected === cat}
          onClick={() => onChange(cat)}
          className={`flex flex-shrink-0 items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium transition-colors ${
            selected === cat
              ? 'bg-primary-50 text-primary-700 border-primary-200'
              : 'bg-white text-stone-600 border-stone-200 hover:bg-stone-50'
          }`}>
          <span
            className={`inline-flex h-4 w-4 items-center justify-center rounded-full ${selected === cat ? skillCategoryChipClassName(cat) : 'bg-stone-100 text-stone-500'}`}>
            <SkillCategoryIcon
              category={cat}
              className={selected === cat ? skillCategoryIconClassName(cat) : 'text-stone-500'}
            />
          </span>
          {cat}
        </button>
      ))}
    </div>
  );
};

export default SkillCategoryFilter;
