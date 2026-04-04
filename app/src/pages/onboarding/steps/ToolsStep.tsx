import { useState } from 'react';

import {
  getDefaultEnabledTools,
  getToolsByCategory,
  TOOL_CATEGORIES,
  type ToolCategory,
} from '../../../utils/toolDefinitions';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface ToolsStepProps {
  onNext: (enabledTools: string[]) => void;
  onBack?: () => void;
}

const CATEGORY_DESCRIPTIONS: Record<ToolCategory, string> = {
  System: 'Shell access and version control',
  Files: 'Read and write files on disk',
  Vision: 'Screen capture and image analysis',
  Web: 'Browser, HTTP, and web search',
  Memory: 'Persistent recall for the AI',
  Automation: 'Cron jobs and scheduled tasks',
};

const ToolsStep = ({ onNext, onBack: _onBack }: ToolsStepProps) => {
  const toolsByCategory = getToolsByCategory();
  const [enabled, setEnabled] = useState<Record<string, boolean>>(() => {
    const defaults: Record<string, boolean> = {};
    const defaultEnabled = getDefaultEnabledTools();
    for (const cat of TOOL_CATEGORIES) {
      for (const tool of toolsByCategory[cat]) {
        defaults[tool.id] = defaultEnabled.includes(tool.id);
      }
    }
    return defaults;
  });

  const toggle = (toolId: string) => {
    setEnabled(prev => ({ ...prev, [toolId]: !prev[toolId] }));
  };

  const enabledList = Object.entries(enabled)
    .filter(([, v]) => v)
    .map(([k]) => k);

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Enable Tools</h1>
        <p className="text-stone-600 text-sm">
          Choose which capabilities OpenHuman can use on your behalf.
        </p>
      </div>

      <div className="space-y-4 mb-5 max-h-[380px] overflow-y-auto pr-1">
        {TOOL_CATEGORIES.map(category => {
          const tools = toolsByCategory[category];
          if (tools.length === 0) return null;
          return (
            <div key={category}>
              <div className="mb-2">
                <h2 className="text-xs font-semibold uppercase tracking-wide text-stone-500">
                  {category}
                </h2>
                <p className="text-xs text-stone-400">{CATEGORY_DESCRIPTIONS[category]}</p>
              </div>
              <div className="space-y-1">
                {tools.map(tool => (
                  <button
                    key={tool.id}
                    type="button"
                    onClick={() => toggle(tool.id)}
                    className="w-full flex items-center justify-between p-2.5 rounded-xl border border-stone-200 bg-white hover:border-stone-300 transition-colors text-left">
                    <div className="min-w-0 flex-1">
                      <span className="text-sm font-medium text-stone-900">{tool.displayName}</span>
                      <p className="text-xs text-stone-500 mt-0.5">{tool.description}</p>
                    </div>
                    <div
                      className={`ml-3 flex-shrink-0 w-9 h-5 rounded-full transition-colors relative ${
                        enabled[tool.id] ? 'bg-sage-500' : 'bg-stone-200'
                      }`}>
                      <div
                        className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                          enabled[tool.id] ? 'translate-x-4' : 'translate-x-0.5'
                        }`}
                      />
                    </div>
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>

      <OnboardingNextButton onClick={() => onNext(enabledList)} />
    </div>
  );
};

export default ToolsStep;
