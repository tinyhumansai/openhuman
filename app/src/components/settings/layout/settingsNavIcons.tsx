import { Fragment, type ReactNode } from 'react';

// ---------------------------------------------------------------------------
// Sidebar icons, keyed by settings registry entry id. Consolidates the SVGs
// previously duplicated across SettingsHome.tsx and Settings.tsx.
// ---------------------------------------------------------------------------

const icon = (path: ReactNode) => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    {path}
  </svg>
);

const stroke = (d: string) => (
  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
);

export const SETTINGS_NAV_ICONS: Record<string, ReactNode> = {
  account: icon(stroke('M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z')),
  appearance: icon(stroke('M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z')),
  theme: icon(
    stroke(
      'M4.098 19.902a3.75 3.75 0 005.304 0l6.402-6.402M6.75 21A3.75 3.75 0 013 17.25V4.125C3 3.504 3.504 3 4.125 3h5.25c.621 0 1.125.504 1.125 1.125v4.072M6.75 21a3.75 3.75 0 003.75-3.75V8.197M6.75 21h13.125c.621 0 1.125-.504 1.125-1.125v-5.25c0-.621-.504-1.125-1.125-1.125h-4.072M10.5 8.197l2.88-2.88c.438-.439 1.15-.439 1.59 0l3.712 3.713c.44.44.44 1.152 0 1.59l-2.879 2.88M6.75 17.25h.008v.008H6.75v-.008z'
    )
  ),
  notifications: icon(
    stroke(
      'M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9'
    )
  ),
  llm: icon(
    stroke(
      'M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z'
    )
  ),
  voice: icon(
    stroke(
      'M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z'
    )
  ),
  personality: icon(
    stroke(
      'M12 21a9 9 0 100-18 9 9 0 000 18zM9 10h.01M15 10h.01M9.5 15c.83.67 1.67 1 2.5 1s1.67-.33 2.5-1'
    )
  ),
  agents: icon(
    stroke(
      'M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2z'
    )
  ),
  profiles: icon(
    stroke(
      'M5.121 17.804A13 13 0 0112 16c2.5 0 4.847.655 6.879 1.804M15 10a3 3 0 11-6 0 3 3 0 016 0zm6 2a9 9 0 11-18 0 9 9 0 0118 0z'
    )
  ),
  devices: icon(
    stroke('M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z')
  ),
  'memory-sync': icon(
    stroke(
      'M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15'
    )
  ),
  'wallet-balances': icon(
    stroke('M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z')
  ),
  integrations: icon(stroke('M13 10V3L4 14h7v7l9-11h-7z')),
  'screen-intelligence': icon(stroke('M3 5h18v12H3zM8 21h8m-4-4v4')),
  'desktop-agent': icon(
    stroke(
      'M9 17v2m6-2v2M5 5h14a1 1 0 011 1v8a1 1 0 01-1 1H5a1 1 0 01-1-1V6a1 1 0 011-1zm4 4l-2 2 2 2m6-4l2 2-2 2'
    )
  ),
  tools: icon(
    stroke(
      'M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065zM15 12a3 3 0 11-6 0 3 3 0 016 0z'
    )
  ),
  companion: icon(
    stroke(
      'M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z'
    )
  ),
  meetings: icon(
    stroke(
      'M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z'
    )
  ),
  'developer-options': icon(stroke('M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4')),
  'keyboard-shortcuts': icon(
    <Fragment>
      {stroke('M4 6h16a1 1 0 011 1v10a1 1 0 01-1 1H4a1 1 0 01-1-1V7a1 1 0 011-1z')}
      {stroke('M7 10h.01M11 10h.01M15 10h.01M17 10h.01M7 13h.01M9 16h6')}
    </Fragment>
  ),
  about: icon(stroke('M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z')),
  usage: icon(
    stroke(
      'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z'
    )
  ),
  billing: icon(
    stroke('M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z')
  ),
  automations: icon(stroke('M13 10V3L4 14h7v7l9-11h-7z')),
  'approval-history': icon(
    stroke(
      'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4'
    )
  ),

  // --- Developer & Diagnostics groups (paths mirror DeveloperOptionsPanel) ---
  // Knowledge & Memory
  intelligence: icon(
    stroke(
      'M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z'
    )
  ),
  'memory-data': icon(
    stroke(
      'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4'
    )
  ),
  'memory-debug': icon(stroke('M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4')),
  'analysis-views': icon(
    stroke(
      'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z'
    )
  ),
  // Agents & Autonomy
  'tool-policy-diagnostics': icon(
    stroke(
      'M9 17v-5a2 2 0 012-2h2a2 2 0 012 2v5m-8 0h8m-8 0H7a2 2 0 01-2-2V7a2 2 0 012-2h10a2 2 0 012 2v8a2 2 0 01-2 2h-2'
    )
  ),
  'agent-chat': icon(
    stroke(
      'M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z'
    )
  ),
  'local-model-debug': icon(
    stroke(
      'M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z'
    )
  ),
  'skills-runner': icon(
    <Fragment>
      {stroke(
        'M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z'
      )}
      {stroke('M21 12a9 9 0 11-18 0 9 9 0 0118 0z')}
    </Fragment>
  ),
  // Models & Inference
  'model-health': icon(
    stroke(
      'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z'
    )
  ),
  agentbox: icon(
    stroke(
      'M21 16V8a2 2 0 00-1-1.73l-7-4a2 2 0 00-2 0l-7 4A2 2 0 003 8v8a2 2 0 001 1.73l7 4a2 2 0 002 0l7-4A2 2 0 0021 16z'
    )
  ),
  'screen-awareness-debug': icon(stroke('M3 5h18v12H3zM8 21h8m-4-4v4')),
  'voice-debug': icon(
    stroke(
      'M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z'
    )
  ),
  'autocomplete-debug': icon(stroke('M4 6h16M4 10h10M4 14h7m3 4h3m0 0l-2-2m2 2l-2 2')),
  // Automation & Integrations
  tasks: icon(
    stroke(
      'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-3 7h3m-6 0h.01M12 16h3m-6 0h.01'
    )
  ),
  'cron-jobs': icon(stroke('M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z')),
  'composio-triggers': icon(stroke('M13 10V3L4 14h7v7l9-11h-7z')),
  'webhooks-debug': icon(
    stroke(
      'M13.828 10.172a4 4 0 010 5.656l-2 2a4 4 0 01-5.656-5.656l1-1m5-5a4 4 0 015.656 5.656l-1 1m-5 5l5-5'
    )
  ),
  'mcp-server': icon(
    stroke('M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z')
  ),
  'dev-workflow': icon(stroke('M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4')),
  search: icon(stroke('M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z')),
  // Diagnostics & Logs
  'event-log': icon(stroke('M4 6h16M4 10h16M4 14h16M4 18h16')),
  'build-info': icon(stroke('M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z')),
};
