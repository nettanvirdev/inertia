/**
 * The things you can type into the message box that are not messages.
 *
 * `/compact`, `/clear`, `/model` - a small vocabulary borrowed from Claude
 * Code, and borrowed for the reason it works there: the composer is already
 * where your hands are, and a person who wants to summarise a conversation
 * should not have to go and find a menu for it.
 *
 * Two rules keep this from eating text somebody meant:
 *
 * - A command is only a command as the FIRST thing in the message. `/usr/bin`
 *   in the middle of a sentence is a path, and a question about `/compact`
 *   is a question.
 * - An unrecognised slash word is not a command at all. `/deploy-notes` is
 *   sent to the agent exactly as typed, because the alternative - an error
 *   saying "unknown command" - would make the box refuse perfectly ordinary
 *   messages that happen to start with a slash.
 *
 * Pure string work and a table, so all of it is testable. What each command
 * DOES lives in the renderer, where the store is; this file only knows the
 * names, so both the palette and the dispatcher read the same list and cannot
 * drift apart.
 */

/**
 * Every command, in the order the palette offers them.
 *
 * Ordered by how often a person reaches for one, not alphabetically: the
 * palette is a list you skim, and `/compact` earning the top row is worth
 * more than the tidiness of `/agents` being first.
 */
export const COMMANDS = [
  {
    name: "compact",
    summary: "Summarise the conversation so far and carry on",
    argHint: "[what to keep]",
    needsThread: true,
  },
  {
    name: "autocompact",
    summary: "Whether long conversations summarise themselves",
    argHint: "[on|off]",
    needsThread: false,
  },
  {
    name: "learn",
    summary: "Write down how to do this, as a skill",
    argHint: "[what, or nothing for this conversation]",
    needsThread: true,
  },
  {
    name: "clear",
    summary: "Start a fresh conversation with this agent",
    argHint: "",
    needsThread: false,
  },
  {
    name: "context",
    summary: "How much of the model's window is in use",
    argHint: "",
    needsThread: true,
  },
  {
    name: "cost",
    summary: "What this conversation has cost so far",
    argHint: "",
    needsThread: true,
  },
  {
    name: "model",
    summary: "Show or change this conversation's model",
    argHint: "[name]",
    needsThread: true,
  },
  { name: "rename", summary: "Rename this conversation", argHint: "<title>", needsThread: true },
  {
    name: "export",
    summary: "Copy the whole conversation to the clipboard",
    argHint: "",
    needsThread: true,
  },
  { name: "stop", summary: "Stop the turn that is running", argHint: "", needsThread: true },
  { name: "agents", summary: "Open the agents screen", argHint: "", needsThread: false },
  { name: "memory", summary: "Open what the agents remember", argHint: "", needsThread: false },
  { name: "permissions", summary: "Open the permission rules", argHint: "", needsThread: false },
  { name: "settings", summary: "Open settings", argHint: "[tab]", needsThread: false },
  {
    name: "help",
    summary: "List the commands this box understands",
    argHint: "",
    needsThread: false,
  },
];

const BY_NAME = new Map(COMMANDS.map((command) => [command.name, command]));

/**
 * Commands that are answered by SENDING something rather than by doing
 * something.
 *
 * `/learn` is the only one so far and it is worth the distinction: it has no
 * machinery of its own, it just puts a carefully written instruction in front
 * of the agent as an ordinary turn. That is why it works everywhere a turn
 * works, and why there is one code path to be wrong rather than two.
 */
export const SENDS_A_TURN = new Set(["learn"]);

/** The command a message is, or `null` when it is an ordinary message. */
export function parseCommand(text) {
  const line = String(text ?? "").trim();
  if (!line.startsWith("/")) return null;
  const match = /^\/([a-z][a-z0-9-]*)(?:\s+([\s\S]*))?$/i.exec(line);
  if (!match) return null;
  const command = BY_NAME.get(match[1].toLowerCase());
  if (!command) return null;
  return { name: command.name, args: (match[2] ?? "").trim(), command };
}

/**
 * The commands worth offering for what has been typed after the slash.
 *
 * A prefix match first, then a match anywhere, which is the same ranking the
 * mention palette uses - "co" should offer `/compact` before `/autocompact`
 * even though both contain it.
 */
export function matchCommands(query) {
  const needle = String(query ?? "")
    .trim()
    .toLowerCase();
  if (!needle) return [...COMMANDS];
  return COMMANDS.map((command) => {
    if (command.name.startsWith(needle)) return { command, rank: 0 };
    if (command.name.includes(needle)) return { command, rank: 1 };
    return null;
  })
    .filter(Boolean)
    .sort((a, b) => a.rank - b.rank)
    .map((hit) => hit.command);
}

/**
 * `on`, `off`, or `null` for "say what it is now".
 *
 * Deliberately generous about the word: someone typing `/autocompact yes`
 * means the same thing as `/autocompact on`, and refusing that would be
 * pedantry rather than safety. Anything that is neither reads as a question.
 */
export function readToggle(args) {
  const word = String(args ?? "")
    .trim()
    .toLowerCase();
  if (!word) return null;
  if (["on", "yes", "true", "enable", "enabled", "1"].includes(word)) return true;
  if (["off", "no", "false", "disable", "disabled", "0"].includes(word)) return false;
  return null;
}

/** The help text, as one block of prose. */
export function helpText() {
  const width = Math.max(...COMMANDS.map((one) => one.name.length + one.argHint.length + 2));
  return COMMANDS.map((one) => {
    const left = `/${one.name}${one.argHint ? ` ${one.argHint}` : ""}`;
    return `${left.padEnd(width + 1)} ${one.summary}`;
  }).join("\n");
}
