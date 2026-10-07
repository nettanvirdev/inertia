/**
 * Watching a turn.
 *
 * The Rust backend runs the agent loop; this is the window's view of it. A
 * turn is started, an id comes back, and everything that happens arrives on
 * one event stream tagged with that id.
 *
 * The events are deliberately close to what actually happened rather than to
 * what the UI wants to draw. Turning "a tool started" into "a card appeared"
 * is the transcript's job, and keeping that translation in one place means the
 * side panel, the activity log and the chat can each render the same turn
 * differently without the bridge growing a shape for each of them.
 */

import { stripSelfLabel } from "@/features/chat/self-label";
import { foldAttachments } from "@/features/chat/attachments";
import { summaryEntry } from "@shared/summary";

/**
 * The bridge, read when it is used rather than when this module was evaluated.
 *
 * The bridge is put on `window` by an import at the top of `main.jsx`, and a
 * module that snapshots the global at import time is one load-order change away
 * from an app that quietly has no tools and no way to say so. Reading it per
 * call costs a property lookup.
 */
const api = () => (typeof window !== "undefined" ? window.agentAPI : null);

export function isAgentAvailable() {
  return Boolean(api()?.run);
}

function unwrap(result) {
  if (!result) throw new Error("The agent bridge did not answer.");
  if (result.ok === false) throw new Error(result.error ?? "The agent bridge failed.");
  return result.data ?? result;
}

/**
 * How long a turn may take to come back with an id.
 *
 * Everything before a turn has an id is setup in the backend: reading the
 * workspace, resolving the provider, unlocking the key, listing the skills.
 * None of it is slow, and all of it is I/O that can block forever on a folder
 * that has gone away - a disconnected network drive, a credential store that
 * never answers. When that happened there was no timer anywhere in the app:
 * the call never settled, the message sat on "typing..." with nothing behind
 * it, and the only way out was to restart, which is exactly how it was
 * reported.
 *
 * Generous, because the setup competes with whatever else main is doing and a
 * false alarm here would abandon a turn that was about to work.
 */
const START_TIMEOUT_MS = 45_000;

/**
 * How long a turn that has started may say nothing before it is checked on.
 *
 * Silence is not failure. A build, a test run or a long file read produces no
 * events for minutes at a time and is perfectly healthy, so this does not end
 * the turn - it asks main whether the turn is still running, and only gives up
 * when the answer is no. That case is real: the last event of a turn can be
 * lost if the window is mid-reload when it is sent, and nothing afterwards
 * would ever have told this message it was finished.
 */
const SILENCE_MS = 120_000;

/**
 * Start a turn and route its events.
 *
 * Returns the two things a caller can do to a turn it does not own: `stop` and
 * `steer`. Both are safe to call before the id has arrived - a user who hits
 * stop during the round trip that starts the turn expects it to stop, and one
 * who types a correction into a turn that began half a second ago expects it to
 * land. Losing either race would leave a turn running with nothing watching it,
 * or silently discard something the person said.
 */
export function runTurn({
  threadId,
  agentId,
  modelRef,
  history,
  messageId,
  mode,
  approval,
  cwd,
  worktree,
  // Group conversations only. The agent the thread belongs to, the agents the
  // person named with `@`, and whether this turn is the next link in a chain
  // the last one started rather than an answer to something just said.
  primaryAgentId,
  mentioned,
  continuation,
  // Who the window last saw in the room, so a backend that has restarted
  // since does not seat the conversation with the primary alone.
  roster,
  // Retry: the one agent that should speak, in place of a turn that failed.
  // Without it the room decides, and the room has already moved on.
  speaker,
  on,
}) {
  const bridge = api();
  if (!bridge?.run) {
    on?.({ type: "error", message: "Tools are only available in the desktop app." });
    return { stop: () => {}, steer: () => Promise.resolve(false) };
  }

  let id = null;
  let cancelled = false;
  let done = false;

  // Resolved once the turn has an id, or once it has failed to get one. Only
  // `steer` waits on it: everything else either buffers (events) or is
  // remembered as a flag and applied when the id lands (cancel).
  let started;
  const whenStarted = new Promise((resolve) => {
    started = resolve;
  });

  /**
   * Events that arrived before we knew what to call this turn.
   *
   * The backend can emit before the call that starts the turn has finished
   * returning its id, and with a fast provider it reliably does: measured
   * against Groq, an entire reply - reasoning, delta and done - landed inside
   * six milliseconds, all of it before the id came back. Dropping those events
   * because the id was still null left the message streaming forever with no
   * error and no content, which is a hang with no way out of it.
   *
   * Holding them costs nothing and the buffer lives for one round trip.
   */
  let pending = [];

  // The two timers that make a turn unable to hang. See the constants above.
  let startTimer = null;
  let silenceTimer = null;

  const clearTimers = () => {
    if (startTimer) clearTimeout(startTimer);
    if (silenceTimer) clearTimeout(silenceTimer);
    startTimer = null;
    silenceTimer = null;
  };

  /** End the turn from this side, when the other side will not. */
  const giveUp = (message) => {
    if (done) return;
    deliver({ type: "error", message });
  };

  /**
   * A turn that has gone quiet: ask whether it is still alive.
   *
   * `agent:active` is the same question a reloaded window asks, and it is the
   * only honest answer available here - the window cannot tell a wedged turn
   * from a slow one by watching, because both look identical.
   */
  const watchSilence = () => {
    if (silenceTimer) clearTimeout(silenceTimer);
    silenceTimer = setTimeout(async () => {
      if (done || !id) return;
      let alive = false;
      try {
        const turns = unwrap(await bridge.active());
        alive = Array.isArray(turns) && turns.some((turn) => turn.id === id);
      } catch {
        // Main did not answer at all, which is worse than a turn that is gone.
      }
      if (done) return;
      if (alive) {
        watchSilence();
        return;
      }
      giveUp("This turn stopped reporting and is no longer running. Send it again.");
    }, SILENCE_MS);
  };

  const deliver = (event) => {
    if (event.type === "done" || event.type === "error") done = true;
    on?.(event);
    if (done) finish();
    else watchSilence();
  };

  const off = bridge.onEvent((event) => {
    if (!id) {
      pending.push(event);
      return;
    }
    if (event.id !== id) return;
    deliver(event);
  });

  function finish() {
    clearTimers();
    off?.();
  }

  startTimer = setTimeout(() => {
    if (id || done) return;
    started();
    giveUp("This turn could not be started. The workspace or the provider did not answer.");
  }, START_TIMEOUT_MS);

  bridge
    .run({
      threadId,
      agentId,
      modelRef,
      history,
      messageId,
      conversationMode: mode,
      approval,
      cwd,
      worktree,
      primaryAgentId,
      mentioned,
      continuation,
      roster,
      speaker,
    })
    .then((result) => {
      const turn = unwrap(result);
      if (done) {
        // The start timer already gave up on this one. The turn is real and
        // running in the backend, so it is stopped rather than left orphaned.
        bridge.cancel(turn.id).catch(() => {});
        return;
      }
      id = turn.id;
      if (startTimer) clearTimeout(startTimer);
      startTimer = null;
      watchSilence();
      started();
      if (cancelled) bridge.cancel(id).catch(() => {});
      on?.({ type: "started", ...turn });

      // Replay in arrival order, so a done that beat the id still ends the turn.
      const held = pending;
      pending = [];
      for (const event of held) {
        if (event.id === id && !done) deliver(event);
      }
    })
    .catch((error) => {
      pending = [];
      started();
      if (done) return;
      finish();
      on?.({ type: "error", message: error?.message ?? String(error) });
    });

  return {
    stop: () => {
      cancelled = true;
      if (id) bridge.cancel(id).catch(() => {});
      finish();
    },
    steer: async (text) => {
      await whenStarted;
      return steerTurn(id, text);
    },
  };
}

/**
 * Hand a message to a turn already in flight.
 *
 * The answer is what the caller needs, not an acknowledgement: `false` means
 * the turn ended before the message got there, and the only right response to
 * that is to send it as an ordinary message rather than to report a failure
 * nobody caused. A bridge that is not there answers the same way, so a browser
 * preview falls back to starting a turn instead of swallowing what was typed.
 */
function steerTurn(id, text) {
  const bridge = api();
  if (!id || !bridge?.steer) return Promise.resolve(false);
  return bridge
    .steer(id, text)
    .then(unwrap)
    .then((result) => Boolean(result?.steered))
    .catch(() => false);
}

/**
 * Turns still running in the backend.
 *
 * A window reload does not stop a turn - the loop, the tools and the key all
 * live in the backend - it only stops anyone watching. This is how a window
 * that has come back finds out what it walked away from.
 */
export function activeTurns() {
  const bridge = api();
  if (!bridge?.active) return Promise.resolve([]);
  return bridge.active().then(unwrap).catch(() => []);
}

/**
 * One turn's record: what has happened in it so far.
 *
 * For a turn this window did not start. A routine's run announces itself
 * with a turn id, and the events that landed between the turn starting and
 * the announcement arriving are in the record and nowhere else.
 */
export function turnRecord(id) {
  const bridge = api();
  if (!id || !bridge?.record) return Promise.resolve(null);
  return bridge.record(id).then(unwrap).catch(() => null);
}

/**
 * Rejoin a turn that was already running.
 *
 * The events that happened while nobody was listening are replayed first, in
 * order, and then the live stream continues from wherever it got to. Both go
 * through the caller's one handler, so the fold that builds the message cannot
 * tell a replayed event from a live one - which is the property that makes a
 * rejoined transcript identical to an uninterrupted one rather than merely
 * similar.
 *
 * A turn that finished in the gap is not a problem: its `done` is in the log,
 * the replay delivers it, and the message settles exactly as it would have.
 */
export function attachTurn({ id, events = [], on }) {
  const bridge = api();
  if (!bridge?.onEvent) return { stop: () => {}, steer: () => Promise.resolve(false) };

  let done = false;
  const deliver = (event) => {
    if (done) return;
    if (event.type === "done" || event.type === "error") done = true;
    on?.(event);
  };

  const off = bridge.onEvent((event) => {
    if (event.id !== id) return;
    deliver(event);
  });

  for (const event of events) {
    if (done) break;
    deliver({ ...event, id });
  }

  if (done) off?.();
  return {
    stop: () => {
      off?.();
      bridge.cancel?.(id).catch(() => {});
    },
    // A rejoined turn is steerable for the same reason it is stoppable: the
    // work never stopped, only the window watching it, and someone who reloads
    // mid-turn has lost none of their right to say "not that file".
    steer: (text) => steerTurn(id, text),
  };
}

/**
 * The prompts that suspend a tool.
 *
 * Permission and question both arrive on the turn's own event channel tagged
 * with `channel`, because a window that subscribed to answers but forgot to
 * subscribe to questions would hang a tool call forever with no sign of why.
 * One subscription, both kinds.
 */
export function watchPrompts(handler) {
  const bridge = api();
  if (!bridge?.onEvent) return () => {};
  return bridge.onEvent((event) => {
    if (event.channel === "permission" || event.channel === "question") handler(event);
  });
}

export const permission = {
  /** `answer` is "once", "always" or "reject". */
  reply: (id, answer, message = "") => api()?.reply(id, answer, message).then(unwrap),
  waiting: (sessionId) => api()?.waiting(sessionId).then(unwrap) ?? Promise.resolve([]),
  /** Every "always" the user has granted this run, flat, with its session. */
  grants: () => api()?.grants().then(unwrap) ?? Promise.resolve([]),
  revoke: (sessionId, tool, pattern) => api()?.revoke(sessionId, tool, pattern).then(unwrap),
};

export const question = {
  waiting: (sessionId) => api()?.questions(sessionId).then(unwrap) ?? Promise.resolve([]),
  answer: (id, value) => api()?.answer(id, value).then(unwrap),
  dismiss: (id) => api()?.dismiss(id).then(unwrap),
};

/**
 * Summarise a conversation because somebody asked, not because it stopped
 * fitting. One call, no tools, the model the conversation is already using.
 */
export function summarize(request) {
  const bridge = api();
  if (!bridge?.summarize) return Promise.reject(new Error("The agent bridge is not available."));
  return bridge.summarize(request).then(unwrap);
}

export const tools = {
  list: () => api()?.tools().then(unwrap) ?? Promise.resolve([]),
  rules: (agentId) => api()?.rules(agentId).then(unwrap) ?? Promise.resolve([]),
  invalidate: () => api()?.invalidate().then(unwrap),
  openPath: (target) => api()?.openPath(target).then(unwrap),
  forget: (sessionId) => api()?.forget(sessionId).then(unwrap),
};

/**
 * The fold lives in `shared/transcript.js` now, because main folds a routine's
 * turn into its transcript with no window involved. Re-exported so nothing that
 * imports it from here has to know.
 */
export { applyEvent, textOf, finishedStatus } from "@shared/transcript";

/**
 * The transcript, flattened back into what a provider understands.
 *
 * A finished turn is stored as parts because that is what the UI renders, but a
 * later turn has to send it as the provider's own shape: an assistant message
 * carrying its tool calls, then one tool message per result. Dropping those and
 * sending only the prose would be simpler and would quietly cost the model
 * everything a tool told it two turns ago - which is exactly the context it
 * needs to answer a follow-up.
 *
 * `label` is for a group conversation, where several agents write into one
 * transcript. Without it the next agent reads an undifferentiated wall of
 * assistant turns and cannot tell its own words from a colleague's - which is
 * the difference between a room and a monologue with several authors. Every
 * agent message is labelled, the reader's own included, because a transcript
 * where only the others are named reads as though the unnamed one is the
 * narrator.
 */
export function toHistory(messages, { label = null } = {}) {
  const out = [];

  for (const message of messages ?? []) {
    /**
     * A compaction, which is a line drawn across the conversation.
     *
     * Everything before it has already been said in the note it carries, so
     * everything before it goes - and it goes here, in the one function that
     * turns a transcript into a request, rather than by deleting messages
     * anybody could still want to scroll back and read. The transcript keeps
     * all of it; the model is sent the note and what came after.
     *
     * A `system` message with no summary is a local note - the answer to
     * `/context`, a word about what a command did - and is not history at
     * all. It is skipped rather than sent as though the agent had said it.
     */
    if (message?.role === "system") {
      if (typeof message.summary === "string" && message.summary.trim()) {
        out.length = 0;
        out.push(summaryEntry(message.summary));
      }
      continue;
    }
    // A turn that passed. It is in the transcript so the reader can see the
    // floor move; it is not in the history, because "pass" is a fact about the
    // room rather than something said, and a model reading a column of them
    // learns to write them.
    if (message?.quiet) continue;
    // Who said it, carried on the entry rather than written into the text:
    // the backend turns the transcript round for whichever agent is
    // about to speak, and it needs the id to know whose lines are whose. A
    // leading "Name:" the model wrote itself comes off, or the next seat reads
    // "Name: Name: ..." and learns it.
    const tag = label?.(message) ?? null;
    const named = (text) => (tag ? stripSelfLabel(text, tag) : text);
    const who = tag ? { agentId: message.agentId, name: tag } : {};
    if (message.role === "user") {
      // Attachments are part of the message, not a second turn.
      //
      // Text files are folded into the content, fenced and named, because a
      // model reads a CSV the same way it reads a paragraph and wants to know
      // which file it is looking at. Images become content parts, which is the
      // only shape any provider accepts them in - the same shape a screenshot
      // from the computer tools already travels in, so the backend needed
      // no new case for this.
      if (message.attachments?.length) {
        const { content, parts } = foldAttachments(message.content ?? "", message.attachments);
        if (parts) out.push({ role: "user", parts });
        else if (content.trim()) out.push({ role: "user", content });
        continue;
      }
      if (message.content?.trim()) out.push({ role: "user", content: message.content });
      continue;
    }

    const parts = message.parts;
    if (!parts?.length) {
      if (message.content?.trim()) out.push({ role: "assistant", content: named(message.content), ...who });
      continue;
    }

    // Parts are in the order they happened, and a tool call has to be followed
    // by its result before the next assistant text. So they are emitted in
    // runs: the text since the last tool, then the calls, then the results.
    let text = "";
    let calls = [];
    let results = [];

    const flush = () => {
      if (!text.trim() && !calls.length) return;
      out.push({
        role: "assistant",
        content: named(text),
        ...(calls.length ? { toolCalls: calls } : {}),
        ...who,
      });
      out.push(...results);
      text = "";
      calls = [];
      results = [];
    };

    for (const part of parts) {
      if (part.type === "text") {
        // Text after a tool result belongs to the next assistant message.
        if (calls.length) flush();
        text += part.text;
      } else if (part.type === "steer") {
        // Interrupting is only worth anything if the interruption keeps its
        // place. Flushed first so everything the model had said and done up to
        // that moment is closed off, then the person speaks, and whatever the
        // model did next follows in a new assistant message - which is the
        // conversation as it actually happened.
        //
        // The mid-turn framing the loop wrapped this in is not repeated. That
        // sentence exists to stop a model treating an interruption as a queued
        // request while it is still working; a turn later it is history, and
        // the plain words are what was said.
        flush();
        if (part.text?.trim()) out.push({ role: "user", content: part.text });
      } else if (part.type === "tool" && part.state !== "running") {
        calls.push({
          id: part.callId,
          name: part.name,
          arguments: JSON.stringify(part.args ?? {}),
        });
        results.push({ role: "tool", toolCallId: part.callId, content: part.output ?? "" });
      }
    }
    flush();
  }

  return out;
}
