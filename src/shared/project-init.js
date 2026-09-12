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

const BY_ID = new Map(PROJECT_INIT.map((mode) => [mode.id, mode]));

export function isProjectInit(id) {
  return BY_ID.has(String(id ?? ""));
}

export function projectInitOf(id) {
  return BY_ID.get(String(id ?? "")) ?? BY_ID.get(DEFAULT_PROJECT_INIT);
}

/**
 * What a new `AGENTS.md` says.
 *
 * Deliberately a skeleton with prompts rather than a guess at the project's
 * conventions. A generated file full of confident inventions is worse than an
 * empty one: it is read on every turn by every agent, so a wrong line in it is
 * a wrong line in every conversation, and nobody edits a file that looks
 * finished.
 */
export function starterAgentsFile(name = "this project") {
  return [
    `# ${name}`,
    "",
    "Notes for anyone, human or agent, working in this repository. This file is",
    "read at the start of every conversation, so keep it short and keep it true -",
    "a stale line here is repeated to everyone, forever.",
    "",
    "## What this is",
    "",
    "<!-- One or two sentences. What the project does and who it is for. -->",
    "",
    "## Running it",
    "",
    "<!-- The commands that matter: install, run, test, build. -->",
    "",
    "## Conventions",
    "",
    "<!-- Things that are true across the project and not obvious from one file. -->",
    "",
    "## Watch out for",
    "",
    "<!-- Known traps, things that look wrong but are deliberate. -->",
    "",
  ].join("\n");
}

/** The one-topic rules folder, with a note saying what it is for. */
export function starterRulesReadme() {
  return [
    "# Rules",
    "",
    "One file per topic. Every `.md` file in this folder is read at the start of",
    "every conversation, in name order, along with `AGENTS.md` above it.",
    "",
    "Use this when `AGENTS.md` starts growing sections that only matter",
    "occasionally - `testing.md`, `deploys.md`, `style.md`. Anything long enough",
    "to need its own document is probably a skill instead, which is loaded only",
    "when it is relevant rather than read every time.",
    "",
  ].join("\n");
}
