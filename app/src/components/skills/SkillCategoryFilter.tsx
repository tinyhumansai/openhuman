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

interface SkillCategoryFilterProps {
  categories: SkillCategory[];
  selected: SkillCategory;
  onChange: (category: SkillCategory) => void;
}

const SkillCategoryFilter = ({
  categories,
  selected,
  onChange,
}: SkillCategoryFilterProps) => {
  return (
    <div className="flex gap-2 overflow-x-auto pb-1 scrollbar-hide">
      {categories.map(cat => (
        <button
          key={cat}
          type="button"
          aria-pressed={selected === cat}
          onClick={() => onChange(cat)}
          className={`flex-shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition-colors ${
            selected === cat
              ? 'bg-primary-50 text-primary-700 border-primary-200'
              : 'bg-white text-stone-600 border-stone-200 hover:bg-stone-50'
          }`}>
          {cat}
        </button>
      ))}
    </div>
  );
};

export default SkillCategoryFilter;
