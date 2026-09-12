import * as React from "react";
import { useWorkspace } from "./workspace";
import {
  asActivityRecords,
  asMessageRecords,
  byTimeDesc,
  byUpdatedDesc,
  fingerprint,
  syncCollection,
  syncDocument, reconcileMessages, mergeRows, settleAgents } from "./workspace-sync.js";
import { asRuleMap, asRules } from "@shared/permission";
import {
  settleMemories,
  settleRoutines,
  settleThreads,
  settleUser,
} from "./settle.js";

/** Long enough to swallow a burst of keystrokes, short enough to feel durable. */
const WRITE_DELAY = 400;

/**
 * The bridge between the app's live state and the workspace folder.
 *
 * The store stays exactly what it was - React state that every screen reads and
 * writes without knowing where the data came from. This hook is the half that
 * makes it durable: it hydrates that state from the folder on open, and mirrors
 * every subsequent change back into it.
 *
 * Everything is driven off one table, `SOURCES`, so wiring a new slice of state
 * to the folder is a row rather than a code path. That matters more than it
 * looks: the sync rules below (what wins, when to seed, what to skip) are subtle
 * enough that a second hand-written copy of them would be a second set of bugs.
 *
 * Three decisions worth stating, because they are the ones that would be
 * annoying to reverse:
 *
 * 1. **The folder wins.** Once a workspace has been seeded, what is on disk is
 *    the truth - including its absences. Deleting `conversations/threads/` from
 *    the file manager has to mean the threads are gone, or the folder is a
 *    cache pretending to be a home.
 *
 * 2. **The first open seeds.** A workspace opens with whatever the app has in
 *    memory written into it, so a new folder is never an empty shell and the
 *    on-disk shapes exist from minute one. A flag in `settings/app.json` makes
 *    that happen exactly once.
 *
 * 3. **Writes are diffed, not dumped.** Sending one message must not rewrite
 *    every thread file. Each source keeps a snapshot of what it last wrote and
 *    only touches the records that actually changed, on a short debounce - a
 *    streaming reply arrives three characters at a time, and that is a repaint
 *    concern, not a disk concern.
 */

/* -- what is wired ----------------------------------------------------- */

/**
 * Every slice of state that lives in the folder.
 *
 * `read` turns the store's shape into records; `apply` turns records back into
 * the store's shape.
 *
 * `executions` is deliberately not here, and used to be. It wrote one
 * record per finished run into `history/executions` with an `apply` that was an
 * empty function and a comment saying nothing read it - a collection that only
 * grew. It was not even a second copy of anything missing: a run's record is
 * written by the scheduler into the routine's own `runHistory`, which is what
 * the routine screen and the run sparkline already read and what survives a
 * restart. Two writers of the same fact, one of them unreadable, is worse than
 * one, so the writer went rather than the reader being invented. Records left
 * in that folder by earlier builds are still listed in the workspace layout and
 * are still cleared by the demo-data purge.
 *
 * `computers` is deliberately NOT here. Its records are written by the main
 * process, which is the only thing that can see whether the container behind
 * one is actually running - and a second writer mirroring stale renderer state
 * over the top would undo a status the provider had just corrected. The screen
 * reads them through the computers bridge instead.
 */
/**
 * Exported for the tests, which drive `apply` directly.
 *
 * What these rules do on a live re-read is the difference between a
 * conversation that is on screen and one that is not, and reaching them
 * through a React hook and a fake IPC client would test the plumbing rather
 * than the rule.
 */
export const SOURCES = [
  {
    collection: "threads",
    // A draft has no file. The folder holds conversations, and a conversation
    // starts at its first message - otherwise every stray New chat would leave
    // an empty JSON file behind for the user to find later and wonder about.
    //
    // Nor does a temporary one, which is the entire content of that promise.
    // The composer offered it for a long time and drew a dashed border for it,
    // and this line is the only place that could ever have made it true.
    read: (s) => (s.threads ?? []).filter((t) => !t.draft && !t.temporary).map((t) => ({ ...t })),
    // Merged, not replaced: a draft has no file, and a conversation written a
    // moment ago may not have reached one yet. Replacing made both disappear
    // from the list whenever anything else wrote to the folder.
    //
    // Settled before the merge, not after. Settling is a rule about what a
    // file may claim - `draft: false`, `temporary: false` - and running it
    // over the merged list applied it to the rows that have no file at all.
    // Every announcement of any workspace write therefore turned this
    // window's drafts into real conversations, which is exactly the empty
    // JSON file the read above exists to avoid.
    apply: (rows, set) =>
      set.setThreads((prev) => byUpdatedDesc(mergeRows(prev, settleThreads(rows)))),
    /**
     * A thread record is never deleted while its messages are still held.
     *
     * The mirror deletes a file when its record has left the state, and for
     * threads there is exactly one legitimate way for that to happen: the
     * person deleted the conversation, which drops the messages in the same
     * action. Every other way a thread can leave the state - a draft pruned,
     * a bug nobody has found yet - leaves its messages behind, and on disk
     * that looked like this: fifteen message files with real conversations in
     * them and an empty threads folder. Fifteen conversations gone from the
     * sidebar with their words still on disk. So a thread whose messages the
     * state still holds is kept, whatever the state says about the thread;
     * the next re-read merges the record back in, and the sidebar heals
     * rather than losing it.
     */
    keep: (s, id) => Boolean(s.messages?.[id]?.length),
  },
  {
    collection: "messages",
    read: (s) => {
      // A thread with no file has no message file either. Saying "this one is
      // not written down" and then writing every message in it under the same
      // id is the worst of the three possible behaviours.
      const unwritten = new Set(
        (s.threads ?? []).filter((t) => t.draft || t.temporary).map((t) => t.id)
      );
      return asMessageRecords(s.messages).filter((row) => !unwritten.has(row.id));
    },
    // Reconciled rather than replaced, because this runs on a live re-read as
    // well as on the initial load, and a live re-read must not be able to
    // declare a turn that is still arriving abandoned.
    apply: (rows, set) =>
      set.setMessages((prev) => {
        // Merged for the same reason the threads are: this replaced the whole
        // map, so every conversation the folder had not heard of yet - the one
        // being typed into, most of all - lost its messages the moment a
        // routine's run, or this window's own write, was announced.
        const next = { ...prev };
        for (const row of rows) next[row.id] = reconcileMessages(prev?.[row.id], row.messages);
        return next;
      }),
  },
  {
    collection: "activity",
    read: (s) => asActivityRecords(s.activity),
    apply: (rows, set) => set.setActivity(byTimeDesc(rows.flatMap((row) => row.events ?? []))),
  },
  {
    collection: "agents",
    read: (s) => (s.agents ?? []).map((a) => ({ ...a })),
    apply: (rows, set) => set.setAgents(settleAgents(rows)),
  },
  {
    collection: "routines",
    read: (s) => (s.routines ?? []).map((r) => ({ ...r })),
    apply: (rows, set) => set.setRoutines(settleRoutines(rows)),
  },
  {
    collection: "memory",
    read: (s) => (s.memories ?? []).map((m) => ({ ...m })),
    apply: (rows, set) => set.setMemories(settleMemories(rows)),
  },
];

/**
 * The two slices that are one document rather than a collection.
 *
 * Permissions are one ruleset plus a map of per-agent overrides, and the user is
 * a single object - both would be a folder holding exactly one file, which is
 * worse to read and worse to hand-edit than the settings file they belong in.
 */
const DOCUMENTS = [
  // Permissions are two fields of one document rather than one blob, because
  // the main process reads `workspace` and `agents` out of it separately and
  // matching that shape here is what keeps the screen and the enforcement
  // talking about the same records.
  {
    document: "settings.permissions",
    field: "workspace",
    read: (s) => s.permissions?.workspace ?? [],
    // Coerced on the way in, never trusted. A folder written by an older build
    // holds `rules` and a map of dead permission ids where this expects an
    // array of rules, and applying that raw put an object into state that every
    // reader spreads - which is how one stale settings file blanked the chat
    // screen and the library with "is not iterable".
    apply: (value, set) => set.setPermissions((prev) => ({ ...prev, workspace: asRules(value) })),
  },
  {
    document: "settings.permissions",
    field: "agents",
    read: (s) => s.permissions?.agents ?? {},
    apply: (value, set) => set.setPermissions((prev) => ({ ...prev, agents: asRuleMap(value) })),
  },
  {
    document: "settings.identity",
    field: "user",
    read: (s) => s.user,
    // The composer reads `user.preferences.sendOnEnter` without a guard, so an
    // identity file hand-edited down to a name and an email stops the user from
    // typing. Every preference is coerced to the type of its own default.
    apply: (value, set) => set.setUser(settleUser(value)),
  },
];

/** Two rows can share one document, so a fingerprint is keyed by the field it
 *  covers rather than by the file it lives in. */
const markOf = (doc) => doc.document + ":" + doc.field;

/**
 * @param state   the store's slices and setters.
 * @param onArrival called with records that appeared from outside this window,
 *   so the shell can say so. Kept as a callback rather than a toast in here
 *   because this module has no opinion about how the app talks to people.
 */
export function useWorkspacePersistence(state, { onArrival } = {}) {
  const { client, configured } = useWorkspace();
  const [hydrated, setHydrated] = React.useState(false);

  // What each source last wrote, so a change can be told from a re-render.
  const written = React.useRef({ collections: {}, documents: {} });

  /**
   * The record objects last handed to the mirror, per collection, by id.
   *
   * Purely a shortcut, and see `syncCollection` for what it saves: a record
   * that is the same object as last time cannot have changed, so it is never
   * hashed. Without it, one streaming reply re-serialised every conversation
   * in the workspace several times a second.
   */
  const seen = React.useRef({});
  const seenFor = (collection) => {
    if (!seen.current[collection]) seen.current[collection] = new Map();
    return seen.current[collection];
  };

  // The live values and setters, read by the debounced writer without making it
  // a dependency - a timer that restarted on every keystroke would never fire.
  const latest = React.useRef(state);
  latest.current = state;

  React.useEffect(() => {
    if (!configured || hydrated) return undefined;
    let alive = true;

    (async () => {
      try {
        const settings = (await client.readDocument("settings.app", {})) ?? {};

        if (settings.seeded) {
          const rows = await Promise.all(
            SOURCES.map((source) => client.list(source.collection))
          );
          const docs = await Promise.all(
            DOCUMENTS.map((doc) => client.readDocument(doc.document, {}))
          );
          if (!alive) return;

          const setters = latest.current;
          SOURCES.forEach((source, i) => {
            // No length guard on purpose. An empty folder means an empty app,
            // because the alternative is demo data reappearing after a delete.
            source.apply(rows[i], setters);
            written.current.collections[source.collection] = new Map(
              rows[i].map((row) => [row.id, fingerprint(row)])
            );
          });

          DOCUMENTS.forEach((doc, i) => {
            // A workspace written by an older build will not have the field.
            // Falling back to what is in memory is kinder than blanking the
            // user's identity because a key was added after they set up.
            const stored = docs[i]?.[doc.field];
            if (stored !== undefined) doc.apply(stored, setters);
            written.current.documents[markOf(doc)] = fingerprint(
              stored !== undefined ? stored : doc.read(setters)
            );
          });
        } else {
          const seed = latest.current;
          for (const source of SOURCES) {
            written.current.collections[source.collection] = await syncCollection(
              client,
              source.collection,
              source.read(seed),
              new Map()
            );
          }
          for (const doc of DOCUMENTS) {
            written.current.documents[markOf(doc)] = await syncDocument(
              client,
              doc.document,
              doc.field,
              doc.read(seed),
              undefined
            );
          }
          await client.writeDocument("settings.app", {
            ...settings,
            seeded: true,
            seededAt: new Date().toISOString(),
          });
        }
      } catch {
        // A workspace that cannot be read is not a reason to lose the session.
        // The app keeps running on what is in memory; the settings pane is
        // where a missing or unreadable folder gets reported.
      } finally {
        if (alive) setHydrated(true);
      }
    })();

    return () => {
      alive = false;
    };
  }, [client, configured, hydrated]);

  // Mirror. Keyed on the state itself so any mutation anywhere in the store
  // reaches the disk without every action having to remember to save.
  React.useEffect(() => {
    if (!configured || !hydrated) return undefined;

    const timer = window.setTimeout(async () => {
      const now = latest.current;
      const marks = written.current;
      try {
        for (const source of SOURCES) {
          marks.collections[source.collection] = await syncCollection(
            client,
            source.collection,
            source.read(now),
            marks.collections[source.collection] ?? new Map(),
            {
              keep: source.keep ? (id) => source.keep(now, id) : undefined,
              seen: seenFor(source.collection),
            }
          );
        }
        for (const doc of DOCUMENTS) {
          marks.documents[markOf(doc)] = await syncDocument(
            client,
            doc.document,
            doc.field,
            doc.read(now),
            marks.documents[markOf(doc)]
          );
        }
      } catch {
        // Leave the snapshots as they are: the next change retries the same
        // records rather than marking them written when they are not.
      }
    }, WRITE_DELAY);

    return () => window.clearTimeout(timer);
  }, [
    client,
    configured,
    hydrated,
    state.threads,
    state.messages,
    state.activity,
    state.agents,
    state.routines,
    state.memories,
    state.permissions,
    state.user,
  ]);

  /**
   * The folder changing under the app.
   *
   * The mirror above is one-way: state goes to disk. Nothing came back, so a
   * record written by anything other than this window - an agent using the
   * `write` tool, an editor, a second window - was invisible until the next
   * launch. An agent that creates an agent is the case that made this obvious,
   * because in this app writing `agents/x.json` is not a side effect of
   * creating an agent, it IS creating one.
   *
   * Only the named collection is re-read, and only after hydration, so this
   * cannot race the initial load. Records this window wrote come back with the
   * fingerprint it recorded and change nothing.
   */
  React.useEffect(() => {
    if (!configured || !hydrated) return undefined;
    if (typeof client.onChanged !== "function") return undefined;

    let alive = true;
    return client.onChanged(async (payload) => {
      /**
       * A document, not a collection.
       *
       * This handler only ever looked at `collection`, so every announcement
       * carrying `document` fell straight through - which is all of them for
       * the permission rules. An agent granting itself `shell(git *)` wrote
       * `settings.permissions` correctly, the Permissions screen kept showing
       * the old ruleset, and then the mirror above wrote the renderer's stale
       * copy back over the file. That is not a display bug: the agent's rules
       * were deleted by the next save of anything.
       */
      const documentKey = payload?.document;
      if (documentKey) {
        const rows = DOCUMENTS.filter((doc) => doc.document === documentKey);
        if (rows.length === 0) return;
        let stored;
        try {
          stored = await client.readDocument(documentKey, {});
        } catch {
          return;
        }
        if (!alive) return;
        for (const doc of rows) {
          const value = stored?.[doc.field];
          if (value === undefined) continue;
          const mark = fingerprint(value);
          if (written.current.documents[markOf(doc)] === mark) continue;
          doc.apply(value, latest.current);
          written.current.documents[markOf(doc)] = mark;
        }
        return;
      }

      const name = payload?.collection;
      const source = SOURCES.find((one) => one.collection === name);
      if (!source) return;

      let rows;
      try {
        rows = await client.list(name);
      } catch {
        return;
      }
      if (!alive) return;

      const before = written.current.collections[name] ?? new Map();
      const after = new Map(rows.map((row) => [row.id, fingerprint(row)]));
      const changed =
        before.size !== after.size ||
        [...after].some(([id, mark]) => before.get(id) !== mark);
      if (!changed) return;

      const appeared = rows.filter((row) => !before.has(row.id));
      source.apply(rows, latest.current);
      written.current.collections[name] = after;
      /**
       * The shortcut is dropped for this collection, deliberately.
       *
       * The marks above now describe what is ON DISK, and the shortcut says
       * "this record object was already written". Those two can disagree the
       * moment something outside this window edits a file: if `apply` merges
       * the change without producing a new object for some record, the next
       * tick would see an unchanged object, carry the disk's mark forward,
       * and never write the in-memory version back. Clearing costs one round
       * of hashing and cannot be wrong.
       */
      delete seen.current[name];
      onArrival?.({ collection: name, rows: appeared });
    });
  }, [client, configured, hydrated, onArrival]);

  return { hydrated };
}
