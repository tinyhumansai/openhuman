import { useMemo, useState } from 'react';

import type { GraphRelation } from '../../utils/tauriCommands';

interface MemoryInsightsProps {
  relations: GraphRelation[];
  loading?: boolean;
}

/**
 * Categorizes graph relations into insight types based on their predicates.
 * This gives the user a structured view of what the system has learned.
 */
type InsightCategory = 'facts' | 'preferences' | 'relationships' | 'skills' | 'opinions' | 'other';

interface InsightGroup {
  category: InsightCategory;
  label: string;
  icon: string;
  color: string;
  bgColor: string;
  borderColor: string;
  items: InsightItem[];
}

interface InsightItem {
  subject: string;
  predicate: string;
  object: string;
  evidenceCount: number;
  namespace: string | null;
  updatedAt: number;
  subjectType: string | null;
  objectType: string | null;
}

const PREDICATE_CATEGORIES: Record<string, InsightCategory> = {
  // Facts
  is: 'facts',
  'is a': 'facts',
  was: 'facts',
  has: 'facts',
  contains: 'facts',
  located_in: 'facts',
  created_by: 'facts',
  founded: 'facts',
  built: 'facts',
  // Preferences
  prefers: 'preferences',
  likes: 'preferences',
  dislikes: 'preferences',
  wants: 'preferences',
  uses: 'preferences',
  favors: 'preferences',
  avoids: 'preferences',
  // Relationships
  knows: 'relationships',
  works_with: 'relationships',
  reports_to: 'relationships',
  manages: 'relationships',
  collaborates_with: 'relationships',
  member_of: 'relationships',
  part_of: 'relationships',
  belongs_to: 'relationships',
  // Skills
  skilled_in: 'skills',
  experienced_with: 'skills',
  certified_in: 'skills',
  specializes_in: 'skills',
  proficient_in: 'skills',
  // Opinions
  thinks: 'opinions',
  believes: 'opinions',
  considers: 'opinions',
  feels: 'opinions',
  views: 'opinions',
};

function categorize(predicate: string): InsightCategory {
  const normalized = predicate.toLowerCase().replace(/[_-]/g, ' ').trim();
  if (PREDICATE_CATEGORIES[normalized]) return PREDICATE_CATEGORIES[normalized];

  // Fuzzy match: check if predicate contains known keywords
  for (const [key, category] of Object.entries(PREDICATE_CATEGORIES)) {
    if (normalized.includes(key) || key.includes(normalized)) return category;
  }

  return 'other';
}

const CATEGORY_CONFIG: Record<
  InsightCategory,
  { label: string; icon: string; color: string; bgColor: string; borderColor: string }
> = {
  facts: {
    label: 'Known Facts',
    icon: 'M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z',
    color: 'text-emerald-600',
    bgColor: 'bg-emerald-50',
    borderColor: 'border-emerald-200',
  },
  preferences: {
    label: 'Preferences',
    icon: 'M4.318 6.318a4.5 4.5 0 000 6.364L12 20.364l7.682-7.682a4.5 4.5 0 00-6.364-6.364L12 7.636l-1.318-1.318a4.5 4.5 0 00-6.364 0z',
    color: 'text-rose-600',
    bgColor: 'bg-rose-50',
    borderColor: 'border-rose-200',
  },
  relationships: {
    label: 'Relationships',
    icon: 'M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z',
    color: 'text-primary-600',
    bgColor: 'bg-primary-50',
    borderColor: 'border-primary-200',
  },
  skills: {
    label: 'Skills & Expertise',
    icon: 'M13 10V3L4 14h7v7l9-11h-7z',
    color: 'text-amber-600',
    bgColor: 'bg-amber-100',
    borderColor: 'border-amber-200',
  },
  opinions: {
    label: 'Opinions & Beliefs',
    icon: 'M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z',
    color: 'text-lavender-600',
    bgColor: 'bg-lavender-50',
    borderColor: 'border-lavender-200',
  },
  other: {
    label: 'Other Insights',
    icon: 'M19.428 15.428a2 2 0 00-1.022-.547l-2.387-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z',
    color: 'text-stone-600',
    bgColor: 'bg-stone-100',
    borderColor: 'border-stone-200',
  },
};

/** Small inline badge that displays an entity type (e.g. "person", "project"). */
function EntityTypeBadge({ type }: { type: string }) {
  return (
    <span className="inline-block ml-1 px-1 py-px rounded text-[9px] leading-tight font-medium bg-white/8 text-stone-400 border border-white/6 uppercase tracking-wide">
      {type}
    </span>
  );
}

export function MemoryInsights({ relations, loading }: MemoryInsightsProps) {
  const [expandedCategory, setExpandedCategory] = useState<InsightCategory | null>(null);

  const groups = useMemo<InsightGroup[]>(() => {
    const buckets = new Map<InsightCategory, InsightItem[]>();

    for (const rel of relations) {
      const category = categorize(rel.predicate);
      const items = buckets.get(category) ?? [];
      const entityTypes = (rel.attrs?.entity_types ?? {}) as Record<string, string>;
      items.push({
        subject: rel.subject,
        predicate: rel.predicate,
        object: rel.object,
        evidenceCount: rel.evidenceCount,
        namespace: rel.namespace,
        updatedAt: rel.updatedAt,
        subjectType: entityTypes.subject ?? null,
        objectType: entityTypes.object ?? null,
      });
      buckets.set(category, items);
    }

    // Sort items within each bucket by evidence count descending
    for (const items of buckets.values()) {
      items.sort((a, b) => b.evidenceCount - a.evidenceCount);
    }

    const categoryOrder: InsightCategory[] = [
      'facts',
      'preferences',
      'relationships',
      'skills',
      'opinions',
      'other',
    ];

    return categoryOrder
      .filter(cat => (buckets.get(cat)?.length ?? 0) > 0)
      .map(cat => ({ category: cat, ...CATEGORY_CONFIG[cat], items: buckets.get(cat)! }));
  }, [relations]);

  if (loading) {
    return (
      <div className="rounded-xl border border-stone-200 bg-stone-50 p-5">
        <h3 className="text-sm font-semibold text-stone-900 mb-4">Intelligent Insights</h3>
        <div className="grid grid-cols-2 lg:grid-cols-3 gap-3">
          {[1, 2, 3].map(i => (
            <div key={i} className="h-28 rounded-lg bg-stone-200 animate-pulse" />
          ))}
        </div>
      </div>
    );
  }

  if (groups.length === 0) {
    return (
      <div className="rounded-xl border border-stone-200 bg-stone-50 p-5">
        <h3 className="text-sm font-semibold text-stone-900 mb-2">Intelligent Insights</h3>
        <p className="text-sm text-stone-600">
          No insights yet. Ingest documents to extract facts, preferences, and relationships.
        </p>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-stone-200 bg-stone-50 p-5">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-semibold text-stone-900">Intelligent Insights</h3>
          <p className="text-xs text-stone-500 mt-0.5">
            Extracted knowledge organized by type — {relations.length} total relations
          </p>
        </div>
      </div>

      <div className="grid grid-cols-2 lg:grid-cols-3 gap-3">
        {groups.map(group => {
          const isExpanded = expandedCategory === group.category;
          const displayItems = isExpanded ? group.items.slice(0, 20) : group.items.slice(0, 3);

          return (
            <div
              key={group.category}
              className={`rounded-lg border ${group.borderColor} ${group.bgColor} p-3 transition-all ${
                isExpanded ? 'col-span-2 lg:col-span-3' : ''
              }`}>
              <button
                onClick={() => setExpandedCategory(isExpanded ? null : group.category)}
                className="flex items-center gap-2 w-full text-left mb-2">
                <div
                  className={`w-7 h-7 rounded-md ${group.bgColor} flex items-center justify-center flex-shrink-0`}>
                  <svg
                    className={`w-4 h-4 ${group.color}`}
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={1.5}
                      d={group.icon}
                    />
                  </svg>
                </div>
                <div className="min-w-0 flex-1">
                  <div className={`text-xs font-medium ${group.color}`}>{group.label}</div>
                  <div className="text-[10px] text-stone-500">{group.items.length} items</div>
                </div>
                <svg
                  className={`w-3.5 h-3.5 text-stone-500 transition-transform ${isExpanded ? 'rotate-180' : ''}`}
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M19 9l-7 7-7-7"
                  />
                </svg>
              </button>

              <div className={`space-y-1.5 ${isExpanded ? 'max-h-80 overflow-y-auto pr-1' : ''}`}>
                {displayItems.map((item, idx) => (
                  <div
                    key={`${item.subject}-${item.predicate}-${item.object}-${idx}`}
                    className="flex items-start gap-1.5 text-[11px] leading-relaxed">
                    <span
                      className="text-stone-900 font-medium shrink-0 max-w-[30%] truncate"
                      title={item.subject}>
                      {item.subject}
                      {item.subjectType && <EntityTypeBadge type={item.subjectType} />}
                    </span>
                    <span className="text-stone-500 shrink-0 italic">{item.predicate}</span>
                    <span className="text-stone-600 truncate" title={item.object}>
                      {item.object}
                      {item.objectType && <EntityTypeBadge type={item.objectType} />}
                    </span>
                    {item.evidenceCount > 1 && (
                      <span className="ml-auto text-[9px] text-stone-600 shrink-0 tabular-nums">
                        x{item.evidenceCount}
                      </span>
                    )}
                  </div>
                ))}
                {!isExpanded && group.items.length > 3 && (
                  <div className="text-[10px] text-stone-500 pt-0.5">
                    +{group.items.length - 3} more
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
