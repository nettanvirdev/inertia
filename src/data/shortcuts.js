/**
 * Every keyboard shortcut the app has, in one list.
 *
 * This is the list the Shortcuts pane prints AND the list the shell binds, on
 * purpose. The reference used to be hand-written prose sitting next to four
 * real `useHotkey` calls, so twenty-three of its rows were claims nothing
 * implemented. One entry per shortcut and one `combo` string per entry is what
 * stops that happening again: the printed chips are formatted from the same
 * string the handler is registered under, so there is nothing left to drift.
 *
 * `scope` says who binds it. "global" is bound once in App.jsx from this list.
 * "composer" is handled inside the message composer, because those keys only
 * mean anything while the editor has focus and a window-level binding would
 * steal them from every other text field in the app.
 *
 * `action` is the handler's name, not a description. App.jsx maps action to
 * function; an entry naming an action nobody handles is a bug, not a feature
 * we have not got to yet.
 */
export const SHORTCUTS = [
  {
    id: 'sc-palette',
    combo: 'mod+k',
    label: 'Open command palette',
    group: 'Navigation',
    action: 'command-palette',
    scope: 'global',
  },
  // The numbers follow the rail top to bottom, so counting rows on screen is
  // the same as reading the shortcut.
  {
    id: 'sc-nav-library',
    combo: 'mod+1',
    label: 'Go to Library',
    group: 'Navigation',
    action: 'view',
    view: 'library',
    scope: 'global',
  },
  {
    id: 'sc-nav-agents',
    combo: 'mod+2',
    label: 'Go to Agents',
    group: 'Navigation',
    action: 'view',
    view: 'agents',
    scope: 'global',
  },
  {
    id: 'sc-nav-computers',
    combo: 'mod+3',
    label: 'Go to Computers',
    group: 'Navigation',
    action: 'view',
    view: 'computers',
    scope: 'global',
  },
  {
    id: 'sc-nav-routines',
    combo: 'mod+4',
    label: 'Go to Routines',
    group: 'Navigation',
    action: 'view',
    view: 'routines',
    scope: 'global',
  },
  {
    id: 'sc-nav-memory',
    combo: 'mod+5',
    label: 'Go to Memory',
    group: 'Navigation',
    action: 'view',
    view: 'memory',
    scope: 'global',
  },
  {
    id: 'sc-nav-integrations',
    combo: 'mod+6',
    label: 'Go to Integrations',
    group: 'Navigation',
    action: 'view',
    view: 'integrations',
    scope: 'global',
  },
  {
    id: 'sc-nav-activity',
    combo: 'mod+7',
    label: 'Go to Activity',
    group: 'Navigation',
    action: 'view',
    view: 'activity',
    scope: 'global',
  },
  {
    id: 'sc-nav-sidebar',
    combo: 'mod+b',
    label: 'Toggle sidebar',
    group: 'Navigation',
    action: 'toggle-sidebar',
    scope: 'global',
  },
  {
    id: 'sc-nav-settings',
    combo: 'mod+,',
    label: 'Open settings',
    group: 'Navigation',
    action: 'settings',
    scope: 'global',
  },

  {
    id: 'sc-thread-new',
    combo: 'mod+n',
    label: 'New chat',
    group: 'Chats',
    action: 'new-thread',
    scope: 'global',
  },
  {
    id: 'sc-thread-next',
    combo: 'mod+j',
    label: 'Next chat',
    group: 'Chats',
    action: 'next-thread',
    scope: 'global',
  },
  {
    id: 'sc-thread-prev',
    combo: 'mod+shift+j',
    label: 'Previous chat',
    group: 'Chats',
    action: 'prev-thread',
    scope: 'global',
  },
  {
    id: 'sc-thread-pin',
    combo: 'mod+shift+p',
    label: 'Pin or unpin this chat',
    group: 'Chats',
    action: 'toggle-pin',
    scope: 'global',
  },

  // The composer keys read off "Send on Enter" rather than being written down
  // twice, because that switch really does swap what Enter means. See
  // `resolveComposerCombo`.
  {
    id: 'sc-composer-send',
    combo: 'enter',
    label: 'Send message',
    group: 'Composer',
    action: 'send-message',
    scope: 'composer',
  },
  {
    id: 'sc-composer-newline',
    combo: 'shift+enter',
    label: 'New line',
    group: 'Composer',
    action: 'newline',
    scope: 'composer',
  },
  {
    id: 'sc-composer-stop',
    combo: 'escape',
    label: 'Stop generating',
    group: 'Composer',
    action: 'stop-generating',
    scope: 'composer',
  },
];

/** The global bindings, in declaration order. */
export const GLOBAL_SHORTCUTS = SHORTCUTS.filter((s) => s.scope === 'global');

/**
 * Enter sends and Shift+Enter breaks the line, unless "Send on Enter" is off,
 * in which case the two swap. Derived rather than stored so the reference
 * cannot print the wrong one at somebody who changed the setting.
 */
function resolveComposerCombo(shortcut, sendOnEnter) {
  if (sendOnEnter) return shortcut.combo;
  if (shortcut.id === 'sc-composer-send') return 'mod+enter';
  if (shortcut.id === 'sc-composer-newline') return 'enter';
  return shortcut.combo;
}

/**
 * The catalogue as the user's own settings make it.
 *
 * Local rather than exported: the reference sheet is the only reader, and it
 * wants the groups. It WAS exported, was deleted for having no importer, and
 * the deletion took the only thing `getShortcutGroups` calls with it - so the
 * Shortcuts tab in Settings threw on render and took the window down to a
 * blank page. Nothing caught it: a name that is not defined is a reference the
 * bundler assumes is a global, and no test opened that tab.
 */
function getShortcuts({ sendOnEnter = true } = {}) {
  if (sendOnEnter) return SHORTCUTS;
  return SHORTCUTS.map((s) =>
    s.scope === 'composer' ? { ...s, combo: resolveComposerCombo(s, false) } : s,
  );
}

/** Shortcuts grouped for the reference sheet, in declaration order. */
export function getShortcutGroups(options) {
  const groups = [];
  for (const shortcut of getShortcuts(options)) {
    let group = groups.find((g) => g.name === shortcut.group);
    if (!group) {
      group = { name: shortcut.group, shortcuts: [] };
      groups.push(group);
    }
    group.shortcuts.push(shortcut);
  }
  return groups;
}
