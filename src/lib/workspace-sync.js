import { asStrings } from "./settle.js";

/**
 * The pure half of workspace persistence.
 *
 * Everything here is a plain function over plain data: turn the store's shapes
 * into records, and write only the records that moved. It is split from the
 * hook in `workspace-store.js` for one reason - these are the rules that would
 * quietly lose a user's data if they were wrong, and a rule you can call from a
 * test is a rule you can be sure of.
 */

/** Activity is a log, so it is stored in day files rather than one per event.
 *  A year of use is 365 readable files instead of tens of thousands. */
export const dayOf = (iso) => String(iso || new Date().toISOString()).slice(0, 10);

/**
 * What a record looked like last time, in sixteen bytes.
 *
 * This was `JSON.stringify(record)`, and the string was KEPT - one per record,
 * for every record of every collection, for as long as the window was open.
 * For most collections that is nothing. For `messages` it is the entire
 * contents of every conversation, pasted screenshots and all, held a second
 * time in a map nobody reads, and re-serialised every four hundred
 * milliseconds for as long as a reply was streaming. A long conversation with
 * two screenshots in it therefore cost tens of megabytes of allocation per
 * second while the agent was talking, which is what a renderer runs out of
 * memory doing.
 *
 * A hash costs the same to compute and keeps nothing. Two independent 32-bit
 * FNV-1a walks plus the length: different multipliers over the same bytes, so
 * a change that collides in one is not going to collide in the other as well,
 * and a change of length cannot collide at all. The failure mode of a
 * collision is one write skipped, and at this width that is not a thing that
 * happens.
 */
export function fingerprint(record) {
  const text = JSON.stringify(record) ?? "";
  let a = 0x811c9dc5;
  let b = 0x01000193;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    a = Math.imul(a ^ code, 0x01000193);
    b = Math.imul(b ^ code, 0x85ebca6b);
  }
  return `${text.length.toString(36)}.${(a >>> 0).toString(36)}.${(b >>> 0).toString(36)}`;
}

/** Newest first, the order every list in the app expects. */
export const byUpdatedDesc = (rows) =>
  [...rows].sort((a, b) => String(b.updatedAt ?? "").localeCompare(String(a.updatedAt ?? "")));

export const byTimeDesc = (rows) =>
  [...rows].sort((a, b) => String(b.at ?? "").localeCompare(String(a.at ?? "")));

/* -- shape conversions ------------------------------------------------- */

/**
 * One record per conversation, and the SAME record while nothing has changed.
 *
 * The identity matters as much as the contents. `syncCollection` skips hashing
 * a record it was handed last time by object identity, and a fresh wrapper
 * built on every tick would defeat that entirely - so the wrapper is cached
 * against the message array it wraps. State here is immutable, so the array
 * changing identity is exactly the definition of the conversation changing.
 *
 * A WeakMap because the key is the array: a conversation the store has dropped
 * takes its cache entry with it, with nothing to remember to clear.
 */
const messageRecords = new WeakMap();

export const asMessageRecords = (messages) =>
  Object.entries(messages ?? {}).map(([threadId, list]) => {
    const rows = list ?? EMPTY_MESSAGES;
    const cached = messageRecords.get(rows);
    // The id is checked, not assumed. Two threads sharing one array is not
    // something this app does, and a cache that would silently write one
    // conversation under the other's name if it ever did is not worth the
    // line it saves.
    if (cached && cached.id === threadId) return cached;
    const record = { id: threadId, threadId, messages: rows };
    messageRecords.set(rows, record);
    return record;
  });

/** One shared empty list, so a thread with no messages also gets a stable key. */
const EMPTY_MESSAGES = [];

/**
 * A reply cannot still be arriving in a process that has only just started.
 *
 * Messages are written to disk as they stream, so a turn that was interrupted -
 * the app was closed, the machine slept, a request hung and was never answered -
 * leaves a message on disk with `status: "streaming"`. Loading that back
 * verbatim restores a lie with real consequences: the composer shows Stop
 * instead of Send, the thread refuses new messages, and the only way out is to
 * find the JSON file by hand. It jams the conversation permanently.
 *
 * So every message is settled on the way in, by the same rule `stopMessage`
 * uses when a person cancels a turn: partial text is kept and marked as
 * stopped, and a reply that produced nothing becomes a visible failure rather
 * than an empty bubble that spins forever.
 *
 * Its tool calls are settled with it. The status was only half of the jam: a
 * card is drawn from its own part's state, and a part left saying `running`
 * is read as work in progress by everything that asks whether a conversation
 * is busy - so a reply marked interrupted still held the composer on Stop.
 */
export function settleOnLoad(messages) {
  let touched = false;
  const settled = (messages ?? []).map((message) => {
    if (message?.status !== "streaming") return message;
    touched = true;
    const partial = String(message.content ?? "").trim();
    return {
      ...message,
      ...(message.parts?.some((part) => part?.type === "tool" && part.state === "running")
        ? {
            parts: message.parts.map((part) =>
              part?.type === "tool" && part.state === "running"
                ? {
                    ...part,
                    state: "failed",
                    output: part.output ?? "This call was interrupted and never finished.",
                  }
                : part
            ),
          }
        : {}),
      status: partial ? "sent" : "error",
      stopped: true,
      error: partial ? message.error : "This reply was interrupted and never finished.",
    };
  });
  // Same array when nothing changed, so hydration does not look like an edit
  // and immediately write every thread back to disk.
  return touched ? settled : messages ?? [];
}

/**
 * Disk state applied over what this window already has.
 *
 * `settleOnLoad` is a rule about starting up, and it was being used as a rule
 * about reading. The two are not the same. The store mirrors messages to disk
 * as they stream, main announces every record it writes, and the window that
 * did the writing hears its own announcement and re-reads - so a turn that was
 * still arriving read its own half-written self back off the disk, saw
 * `status: "streaming"`, and concluded a turn had been abandoned. The reply
 * died mid-sentence with "This reply was interrupted and never finished." while
 * the request it was describing was still streaming perfectly well underneath.
 * Pressing Retry worked because the second turn happened to win the race.
 *
 * So a message this window is holding wins over the copy on disk. Disk is
 * behind by construction - it is written from memory, on a timer - and the only
 * `streaming` message it can hold that is genuinely stale is one this window
 * has never heard of, which is exactly the abandoned turn the settling rule was
 * written for.
 */
export function reconcileMessages(previous, incoming) {
  const list = incoming ?? [];
  if (!previous?.length) return settleOnLoad(list);

  const mine = new Map(previous.map((message) => [message?.id, message]));
  const settled = list.map((message) => {
    if (message?.status !== "streaming") return message;
    const held = mine.get(message.id);
    // Held at all, not held-and-still-streaming: a turn that finished a moment
    // ago is also fresher in memory than the last thing written down for it.
    return held ?? settleOnLoad([message])[0];
  });

  // A message that has not reached the disk yet is not a message that does not
  // exist. Disk is behind by construction - it is written from memory, on a
  // timer that every keystroke of a streaming reply pushes further out - and a
  // re-read can land at any moment, including the moment between a question
  // being asked and the file being written.
  //
  // This used to keep only the `streaming` ones, which meant the question
  // itself was dropped: the conversation on screen lost the message that had
  // just been sent while the reply to it carried on underneath. Anything this
  // window is holding that disk has not heard of is kept, in the order it was
  // held, which is the newest end of the thread by construction.
  const known = new Set(list.map((message) => message?.id));
  const unwritten = previous.filter((message) => message?.id && !known.has(message.id));
  return unwritten.length ? [...settled, ...unwritten] : settled;
}

/**
 * Disk's rows, without dropping the rows only this window has.
 *
 * `apply` runs on a live re-read as well as on the initial load, and it used to
 * replace the whole list with whatever the folder held. Everything that has not
 * been written yet therefore disappeared the instant anything else touched the
 * collection - and something else touches it constantly: the folder watcher
 * announces every write, including a routine's, and the mirror only writes on
 * a timer that a streaming reply keeps pushing back.
 *
 * What that looked like was a new conversation whose first message vanished on
 * send, leaving the starter screen with the thread named in the sidebar, and a
 * reply that flickered into an error and back. Both were one rule: a re-read
 * may correct what this window holds, and may not delete it.
 */
export function mergeRows(previous, rows, { id = (row) => row?.id } = {}) {
  const list = rows ?? [];
  if (!previous?.length) return list;
  const known = new Set(list.map(id));
  const missing = previous.filter((row) => id(row) && !known.has(id(row)));
  return missing.length ? [...list, ...missing] : list;
}

/**
 * An agent record, brought up to date on the way in.
 *
 * `capabilities` was a list of seven invented names - Browser, Desktop, Voice
 * and Web search among them - that named no tool this app has ever had and that
 * nothing in the main process ever read. It is dropped rather than translated,
 * because translating it would invent a meaning it never carried: an agent that
 * claimed "browser" was not granted anything, so removing the claim takes
 * nothing away. What an agent may do is its permission rules, and an agent with
 * none inherits the workspace ruleset, which is the honest default.
 *
 * The same pass drops a model the workspace cannot reach. Every seeded agent
 * names one from the old demo catalogue, and a stored id that resolves to
 * nothing is a picker showing a blank and a turn quietly falling back at send
 * time. Empty means "follow the workspace", which is what was happening anyway.
 */
export function settleAgent(agent, knownModels) {
  if (!agent || typeof agent !== "object") return agent;
  const stale = "capabilities" in agent;
  const unknownModel =
    Boolean(agent.model) &&
    Array.isArray(knownModels) &&
    knownModels.length > 0 &&
    !knownModels.includes(agent.model);
  // `(agent.tags ?? []).length` guards the check and `agent.tags.map` runs
  // unguarded on the next line, so a record whose tags are a string passes the
  // guard and throws on the map. The list is a list or it is nothing.
  const badTags = "tags" in agent && !Array.isArray(agent.tags);

  // An agent that has never done anything has never been active.
  //
  // A record written by hand - or by another agent, which is how agents get
  // made here - carries whatever timestamp its author felt like. One arrived
  // stamped three hours before it existed, so a brand new agent introduced
  // itself as "active 3h ago". What can be checked is the work it has been
  // credited with, and none of it means there is nothing to report.
  //
  // All three counters, not only the messages. An agent that exists to run a
  // routine every morning answers nobody and had its "active 2m ago" wiped on
  // every launch by a rule that only knew about conversations.
  const worked =
    Number(agent.stats?.messages) > 0 ||
    Number(agent.stats?.routinesRun) > 0 ||
    Number(agent.stats?.tokensUsed) > 0;
  const invented = Boolean(agent.lastActiveAt) && !worked;

  if (!stale && !unknownModel && !badTags && !invented) return agent;

  const { capabilities, ...rest } = agent;
  if (badTags) rest.tags = asStrings(agent.tags);
  if (invented) rest.lastActiveAt = null;
  return unknownModel ? { ...rest, model: "" } : rest;
}

export function settleAgents(agents, knownModels) {
  let touched = false;
  const settled = (agents ?? []).map((agent) => {
    const next = settleAgent(agent, knownModels);
    if (next !== agent) touched = true;
    return next;
  });
  // Same array when nothing changed, so hydration does not look like an edit.
  return touched ? settled : agents ?? [];
}

export function asActivityRecords(activity) {
  const days = new Map();
  for (const event of activity ?? []) {
    const day = dayOf(event.at);
    if (!days.has(day)) days.set(day, []);
    days.get(day).push(event);
  }
  return [...days.entries()].map(([day, events]) => ({ id: day, date: day, events }));
}

/* -- the sync primitives ------------------------------------------------ */

/**
 * Write only what moved.
 *
 * `previous` is the map of id -> fingerprint this source last wrote. A record
 * whose fingerprint is unchanged is skipped; one that has vanished from the
 * incoming list is deleted from the folder, which is what makes the folder the
 * truth rather than a cache that only grows.
 *
 * There used to be an append-only mode, for a collection whose records were
 * written and never read. That collection is gone, and a mode with no caller is
 * a rule nobody can check, so it went with it.
 */
export async function syncCollection(client, name, records, previous, { keep, seen } = {}) {
  const next = new Map();
  const writes = [];

  for (const record of records) {
    /**
     * The record we were handed last time, by identity.
     *
     * Hashing is cheap per byte and a conversation is a lot of bytes, so the
     * cheapest hash is the one not computed. State here is immutable - every
     * change to a thread's messages produces a new array and a new record -
     * so a record that is the same OBJECT as the one written last time cannot
     * have changed, and the mark carries over untouched. While one
     * conversation streams, this is what keeps the other forty from being
     * serialised every four hundred milliseconds.
     *
     * `seen` is optional and is maintained here. A caller that does not pass
     * one gets the old behaviour, which is correct and slower.
     */
    if (seen && seen.get(record.id) === record && previous.has(record.id)) {
      next.set(record.id, previous.get(record.id));
      continue;
    }
    const mark = fingerprint(record);
    next.set(record.id, mark);
    seen?.set(record.id, record);
    if (previous.get(record.id) !== mark) writes.push(client.put(name, record));
  }

  for (const [id, mark] of previous) {
    if (next.has(id)) continue;
    // A record the state has lost but that `keep` vouches for is left on
    // disk, and its mark is carried forward so the next tick does not try
    // again. See the threads source for the one case this exists for.
    if (keep?.(id)) {
      next.set(id, mark);
      continue;
    }
    // A record that has left the state must leave the shortcut too, or a
    // deleted conversation's messages are held by this map for as long as the
    // window is open - which is the leak this whole change exists to close,
    // reintroduced by the fix for it.
    seen?.delete(id);
    writes.push(client.remove(name, id));
  }

  if (!writes.length) {
    // The marks are unchanged, but `seen` has just learnt this round's record
    // objects - which is the whole point of it, and is why the early return
    // stays here rather than above the loop.
    return previous;
  }
  await Promise.all(writes);
  return next;
}

export async function syncDocument(client, key, field, value, previousMark) {
  const mark = fingerprint(value);
  if (mark === previousMark) return previousMark;
  const current = (await client.readDocument(key, {})) ?? {};
  await client.writeDocument(key, { ...current, [field]: value });
  return mark;
}
