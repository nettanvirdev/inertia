/*
 * The name an agent writes in front of its own reply, taken back off.
 *
 * In a group, every reply in the transcript the model reads is labelled with
 * who wrote it - that is how the next agent tells its own words from a
 * colleague's. The prompt says not to write the label yourself, and the
 * stronger models listen; the weaker ones copy what they see and open with
 * "Folder Organizer: Hi." The name and the face are already on the bubble, so
 * on screen that is the name twice, and in the next turn's transcript it would
 * be "Folder Organizer: Folder Organizer: Hi" - which the model would then
 * learn from. Both places strip it, with the same function.
 */

function escape(name) {
  return String(name).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * `text` without a leading "Name:" - or "**Name:**", or "Name -" - and the
 * whitespace after it. Only at the very start: a name in the middle of a
 * sentence is the agent talking about somebody.
 */
export function stripSelfLabel(text, name) {
  if (!name || !text) return text ?? "";
  // The bold form comes both ways: "**Nova:** hi" and "**Nova**: hi".
  const label = new RegExp(
    `^\\s*(?:\\*\\*)?${escape(name)}(?:\\*\\*)?\\s*[:\\-–—]\\s*(?:\\*\\*)?\\s*`,
    "i"
  );
  return String(text).replace(label, "");
}

/**
 * The same, applied to a reply's parts: only the first text part, only at its
 * start, and only if that text part is the first thing the reply said.
 */
export function stripSelfLabelParts(parts, name) {
  return mapFirstText(parts, (text) => stripSelfLabel(text, name));
}

function mapFirstText(parts, fn) {
  if (!parts?.length) return parts;
  const first = parts.findIndex((part) => part?.type === "text");
  if (first === -1) return parts;
  const changed = fn(parts[first].text);
  if (changed === parts[first].text) return parts;
  return parts.map((part, i) => (i === first ? { ...part, text: changed } : part));
}

/**
 * Where a reply stops being this agent and starts being a script.
 *
 * The first real group chat ended with one agent writing the whole discussion
 * itself - "Inertia Dev: ... Folder Organizer: ... Inertia Dev: ..." - in a
 * single reply. The transcript is now shown to each agent from its own seat,
 * which is what stops the habit forming; this is the guard for the model that
 * does it anyway. A line that opens with a colleague's name and a colon is the
 * start of a script, and nothing from that line on was this agent speaking.
 *
 * Returns the index of that line's start, or -1. Only a name from `names`
 * counts, and only at the start of a line, so an agent quoting somebody
 * mid-sentence is left alone.
 */
export function scriptStart(text, names = []) {
  const value = String(text ?? "");
  const list = (names ?? []).filter(Boolean);
  if (!value || !list.length) return -1;
  const label = new RegExp(
    `(?:^|\\n)[ \\t]*(?:\\*\\*)?(?:${list.map(escape).join("|")})(?:\\*\\*)?[ \\t]*:`,
    "i"
  );
  const match = label.exec(value);
  if (!match) return -1;
  return match.index + (value[match.index] === "\n" ? 1 : 0);
}

/** `text` up to the first line written for somebody else. */
export function cutScript(text, names = []) {
  const at = scriptStart(text, names);
  return at === -1 ? String(text ?? "") : String(text ?? "").slice(0, at).trimEnd();
}

/**
 * The same, on a reply's parts: the text part the script starts in is cut
 * there and everything after it is dropped, tool calls included - a tool the
 * agent called while speaking as somebody else was part of the script.
 */
export function cutScriptParts(parts, names = []) {
  if (!parts?.length) return parts;
  for (let i = 0; i < parts.length; i += 1) {
    const part = parts[i];
    if (part?.type !== "text") continue;
    const at = scriptStart(part.text, names);
    if (at === -1) continue;
    const kept = String(part.text).slice(0, at).trimEnd();
    return [...parts.slice(0, i), ...(kept ? [{ ...part, text: kept }] : [])];
  }
  return parts;
}

/**
 * A reply that is the agent choosing not to speak.
 *
 * In a group, "pass" on its own means "nothing to add", and is dropped rather
 * than shown - a bubble that says "pass" is exactly as much noise as the
 * paragraph of agreement it replaced.
 */
export function isPass(text) {
  return /^\s*(?:\*\*)?pass(?:\*\*)?[.!]?\s*$/i.test(String(text ?? ""));
}
