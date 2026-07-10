import { hotkeyManager } from './hotkeyManager';
import { registry } from './registry';
import { isMac } from './shortcut';

// Group headings shown (in this order) by the command palette and the
// keyboard-shortcuts help directory. Kept in sync with the `group` values
// assigned to each action below.
export const GROUP_ORDER = ['Navigation', 'Profiles', 'Chat', 'View', 'General'] as const;

/**
 * i18n keys for the group headings surfaced in the command palette / shortcuts
 * directory. Grouping/ordering keys off the stable English `group` string; the
 * heading text is resolved through `useT()` at display time via this map.
 */
export const GROUP_LABEL_KEYS: Record<string, string> = {
  Navigation: 'shortcuts.group.navigation',
  Profiles: 'shortcuts.group.profiles',
  Chat: 'shortcuts.group.chat',
  View: 'shortcuts.group.view',
  General: 'shortcuts.group.general',
  Help: 'shortcuts.group.help',
};

/**
 * Navigation tabs are bound to the **Control** key.
 *
 * On macOS `ctrl+N` is the physical Control key (distinct from ⌘), which is
 * exactly what we want so ⌘+N can later mean "switch profile". On Windows/Linux
 * there is no ⌘ and our matcher treats a bare `ctrl+N` as unreachable (mod *is*
 * Ctrl there), so nav folds to `mod+N` — i.e. Ctrl+N physically on every OS.
 */
const NAV_MOD = isMac() ? 'ctrl' : 'mod';

/**
 * Profile-switch shortcuts (⌘1–⌘4 on macOS) are wired but inert: profiles don't
 * exist yet. While this is false the actions stay bound-but-disabled and hidden
 * from the palette/help directory; flip it true (and supply a real handler) when
 * the profiles feature lands.
 */
const PROFILES_ENABLED = false;

/**
 * Side-effecting handlers the global command layer drives. The owning
 * `CommandProvider` supplies these (navigation, new chat, sidebar toggle, and
 * the two provider-level overlays) so the shortcut map itself stays declarative
 * and testable here, decoupled from React state.
 */
export interface GlobalActionHandlers {
  navigate: (path: string) => void;
  newChat: () => void;
  toggleSidebar: () => void;
  openPalette: () => void;
  openShortcuts: () => void;
}

interface BindCombo {
  combo: string;
  /** Allow this combo to fire while a text input / editable is focused. */
  allowInInput?: boolean;
}

interface GlobalActionDef {
  id: string;
  label: string;
  /** i18n key for the label, resolved at display time (falls back to `label`). */
  labelKey?: string;
  group: (typeof GROUP_ORDER)[number] | 'Help';
  /** Primary shortcut — shown in the palette / help list and bound. */
  shortcut: string;
  /** `allowInInput` for the primary shortcut's binding. */
  allowInInput?: boolean;
  /** Extra key combos bound to the same handler (not shown separately). */
  aliases?: BindCombo[];
  /** Gate for both the binding (won't fire) and palette/help visibility. */
  enabled?: () => boolean;
  /**
   * When false the combo is still bound (so it's "wired") but NOT surfaced in
   * the palette / help directory. Used for placeholder shortcuts. Default true.
   */
  register?: boolean;
  handler: () => void;
  keywords?: string[];
}

/**
 * Builds the canonical list of global actions from the supplied handlers. This
 * is the single source of truth for the app-wide shortcut map: the command
 * palette, the keyboard-shortcuts help directory, and the actual key bindings
 * all derive from it.
 */
function buildGlobalActions(h: GlobalActionHandlers): GlobalActionDef[] {
  const nav = (path: string) => () => {
    h.navigate(path);
  };

  // Placeholder until the profiles feature exists; the index is captured so the
  // real switch can drop straight in here later.
  const switchProfile = (index: number) => () => {
    void index;
  };

  const profileActions: GlobalActionDef[] = [1, 2, 3, 4].map(n => ({
    id: `profile.switch-${n}`,
    label: `Switch to Profile ${n}`,
    group: 'Profiles' as const,
    // ⌘N on macOS (mod === ⌘); Ctrl+N on Windows/Linux. Distinct from the
    // Control-based nav row on macOS; harmlessly inert elsewhere while disabled.
    shortcut: `mod+${n}`,
    enabled: () => PROFILES_ENABLED,
    register: PROFILES_ENABLED,
    handler: switchProfile(n),
    keywords: ['profile', 'switch', 'persona', `profile ${n}`],
  }));

  return [
    // ── Navigation (Control-based) ──────────────────────────────────────
    {
      id: 'nav.home',
      label: 'Go Home',
      labelKey: 'shortcuts.action.home',
      group: 'Navigation',
      shortcut: `${NAV_MOD}+1`,
      handler: nav('/home'),
      keywords: ['dashboard'],
    },
    {
      id: 'nav.chat',
      label: 'Go to Chat',
      labelKey: 'shortcuts.action.chat',
      group: 'Navigation',
      shortcut: `${NAV_MOD}+2`,
      handler: nav('/chat'),
      keywords: ['conversations', 'messages', 'inbox'],
    },
    {
      id: 'nav.intelligence',
      label: 'Go to Knowledge & Memory',
      labelKey: 'shortcuts.action.knowledge',
      group: 'Navigation',
      shortcut: `${NAV_MOD}+3`,
      handler: nav('/settings/intelligence'),
      keywords: ['memory', 'knowledge', 'intelligence'],
    },
    {
      id: 'nav.skills',
      label: 'Go to Connections',
      labelKey: 'shortcuts.action.connections',
      group: 'Navigation',
      shortcut: `${NAV_MOD}+4`,
      handler: nav('/connections'),
      keywords: ['plugins', 'tools', 'connections', 'apps', 'skills'],
    },
    {
      id: 'nav.activity',
      label: 'Go to Activity',
      labelKey: 'shortcuts.action.activity',
      group: 'Navigation',
      shortcut: `${NAV_MOD}+5`,
      handler: nav('/activity'),
      keywords: ['tasks', 'automations', 'alerts', 'background'],
    },
    {
      id: 'nav.settings',
      label: 'Open Settings',
      labelKey: 'shortcuts.action.settings',
      group: 'Navigation',
      // Settings keeps the conventional ⌘, (mod) — it isn't a numbered tab.
      shortcut: 'mod+,',
      handler: nav('/settings'),
      keywords: ['preferences', 'config'],
    },

    // ── Profiles (wired, hidden until the feature exists) ───────────────
    ...profileActions,

    // ── Chat ────────────────────────────────────────────────────────────
    {
      id: 'chat.new',
      label: 'New Chat',
      labelKey: 'shortcuts.action.newChat',
      group: 'Chat',
      shortcut: 'mod+n',
      // Must fire from the composer too — that's the normal focus state when a
      // user decides to start fresh — and preventDefault keeps the OS "new
      // window" accelerator from swallowing it.
      allowInInput: true,
      handler: h.newChat,
      keywords: ['new', 'thread', 'compose', 'conversation', 'session'],
    },

    // ── View ────────────────────────────────────────────────────────────
    {
      id: 'view.toggle-sidebar',
      label: 'Toggle Sidebar',
      labelKey: 'shortcuts.action.toggleSidebar',
      group: 'View',
      shortcut: 'mod+b',
      handler: h.toggleSidebar,
      keywords: ['sidebar', 'panel', 'rail', 'hide', 'show', 'collapse', 'expand'],
    },

    // ── General ─────────────────────────────────────────────────────────
    {
      id: 'meta.command-palette',
      label: 'Command Palette',
      labelKey: 'shortcuts.action.commandPalette',
      group: 'General',
      shortcut: 'mod+k',
      allowInInput: true,
      aliases: [{ combo: 'mod+p', allowInInput: true }],
      handler: h.openPalette,
      keywords: ['command', 'palette', 'search', 'actions', 'run'],
    },
    {
      id: 'meta.keyboard-shortcuts',
      label: 'Keyboard Shortcuts',
      labelKey: 'shortcuts.title',
      group: 'General',
      // `mod+/` is allowed in inputs so it can replace the command palette
      // while its search box is focused; the bare `?` alias is not, so it
      // never hijacks a literal "?" typed into a message.
      shortcut: 'mod+/',
      allowInInput: true,
      aliases: [{ combo: '?', allowInInput: false }],
      handler: h.openShortcuts,
      keywords: ['keyboard', 'shortcuts', 'keys', 'hotkeys', 'help', 'cheatsheet'],
    },
  ];
}

/**
 * Registers every global action (palette entries + key bindings) against the
 * provided global scope frame. Returns a disposer that unwinds all of them.
 */
export function registerGlobalActions(
  handlers: GlobalActionHandlers,
  globalScopeSymbol: symbol
): () => void {
  const actions = buildGlobalActions(handlers);

  const disposers: Array<() => void> = [];
  for (const a of actions) {
    // `register: false` actions are still bound (wired) but stay out of the
    // palette / help directory.
    const disposeRegistry =
      a.register === false
        ? () => {}
        : registry.registerAction(
            {
              id: a.id,
              label: a.label,
              labelKey: a.labelKey,
              group: a.group,
              shortcut: a.shortcut,
              handler: a.handler,
              keywords: a.keywords,
              allowInInput: a.allowInInput,
              enabled: a.enabled,
            },
            globalScopeSymbol
          );

    const combos: BindCombo[] = [
      { combo: a.shortcut, allowInInput: a.allowInInput },
      ...(a.aliases ?? []),
    ];
    const bindingSyms: symbol[] = [];
    for (const { combo, allowInInput } of combos) {
      bindingSyms.push(
        hotkeyManager.bind(globalScopeSymbol, {
          shortcut: combo,
          handler: a.handler,
          allowInInput,
          enabled: a.enabled,
          id: `${a.id}:${combo}`,
        })
      );
    }

    disposers.push(() => {
      disposeRegistry();
      for (const sym of bindingSyms) hotkeyManager.unbind(globalScopeSymbol, sym);
    });
  }

  return () => {
    for (const d of disposers) d();
  };
}
