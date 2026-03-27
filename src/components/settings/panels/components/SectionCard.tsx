import { ChevronDownIcon, ChevronRightIcon } from '@heroicons/react/24/outline';
import React, { useState } from 'react';

interface SectionCardProps {
  title: string;
  priority: 'critical' | 'infrastructure' | 'development' | 'tools';
  icon: React.ReactElement;
  children: React.ReactNode;
  collapsible?: boolean;
  defaultExpanded?: boolean;
  hasChanges?: boolean;
  loading?: boolean;
}

const priorityStyles = {
  critical: 'bg-gradient-to-br from-primary-500/10 to-primary-600/5 border-primary-500/20',
  infrastructure: 'bg-gradient-to-br from-slate-500/8 to-slate-600/4 border-slate-500/15',
  development: 'bg-gradient-to-br from-amber-500/8 to-amber-600/4 border-amber-500/15',
  tools: 'bg-black/30 border-stone-600/30',
} as const;

const priorityIconColors = {
  critical: 'text-primary-400',
  infrastructure: 'text-slate-400',
  development: 'text-amber-400',
  tools: 'text-stone-400',
} as const;

const SectionCard: React.FC<SectionCardProps> = ({
  title,
  priority,
  icon,
  children,
  collapsible = false,
  defaultExpanded = true,
  hasChanges = false,
  loading = false,
}) => {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);

  const handleToggle = () => {
    if (collapsible) {
      setIsExpanded(!isExpanded);
    }
  };

  return (
    <div
      className={`rounded-xl border backdrop-blur-sm transition-all duration-200 ${priorityStyles[priority]}`}>
      <div
        className={`flex items-center justify-between p-6 ${collapsible ? 'cursor-pointer hover:bg-white/5' : ''}`}
        onClick={handleToggle}>
        <div className="flex items-center gap-3">
          <div
            className={`flex-shrink-0 ${priorityIconColors[priority]} ${loading ? 'relative' : ''}`}>
            {loading ? (
              <div className="h-5 w-5 border-2 border-white/20 border-t-white rounded-full animate-spin" />
            ) : (
              React.cloneElement(icon as React.ReactElement<{ className?: string }>, {
                className: 'h-5 w-5',
              })
            )}
          </div>
          <div className="flex items-center gap-2">
            <h3 className="text-xl font-semibold text-white font-display">{title}</h3>
            {hasChanges && <div className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />}
            {loading && <span className="text-sm text-gray-400 ml-2">Loading...</span>}
          </div>
        </div>
        {collapsible && (
          <div className="text-gray-400 transition-transform duration-200">
            {isExpanded ? (
              <ChevronDownIcon className="h-5 w-5" />
            ) : (
              <ChevronRightIcon className="h-5 w-5" />
            )}
          </div>
        )}
      </div>

      {(!collapsible || isExpanded) && (
        <div className="px-6 pb-6">
          <div className="space-y-8">{children}</div>
        </div>
      )}
    </div>
  );
};

export default SectionCard;
