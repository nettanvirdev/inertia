/**
 * When a project gets its instruction files.
 *
 * `AGENTS.md` and `.inertia/rules/` are read on every turn from the repository
 * root down to the working folder. They are the most direct way to tell every
 * agent how a project works, and they are plain files in the repository, so
 * they belong to the project rather than to this app - Claude Code, Codex and
 * anything else that reads `AGENTS.md` benefits from the same file.
 *
 * Which is exactly why creating one is a decision and not a side effect. Writing
 * into someone's repository uninvited is the sort of thing that shows up in a
 * diff three days later and is not welcome. So there is a setting, the safe
 * option is the default, and every mode asks before the first write.
 */

export const PROJECT_INIT = [
  {
    id: "ask",
    label: "Offer when a project has none",
    hint: "A quiet prompt the first time you work in a folder with no AGENTS.md. Nothing is written until you say so.",
  },
  {
    id: "folder",
    label: "When a folder is opened",
    hint: "Set it up as soon as you point a conversation at a project.",
  },
  {
    id: "message",
    label: "On the first message",
    hint: "Set it up when you actually start work, so a folder you only glanced at stays untouched.",
  },
  {
    id: "manual",
    label: "Never automatically",
    hint: "Only when you ask for it. Nothing is ever created on its own.",
  },
];

/**
 * Asking, rather than doing it silently.
 *
 * A file appearing in a repository nobody asked to change is worse than a file
 * that is missing, because the missing one is noticed and the surprising one is
 * committed.
 */
export const DEFAULT_PROJECT_INIT = "ask";
