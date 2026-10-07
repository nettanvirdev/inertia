/**
 * A turn's events, folded into the message being written.
 *
 * The window folds a live turn into the reply on screen with this. The Rust
 * backend builds a routine's reply on disk with its own fold (`reply.rs`),
 * because a routine runs whether or not a window is open and a transcript that
 * only exists while somebody is watching is not a transcript. The two must
 * agree about what a tool card looks like.
 *
 * The ordering rules - a tool card appears the moment its call starts, not
 * when it finishes, and text written after a tool call belongs after that card
 * - are the difference between a transcript that reads like a session and one
 * that reads like a log.
 */

/**
 * No call is still running once the turn is over.
 *
 * A tool card is drawn from its part's `state`, and the only thing that ever
 * moved a part out of `running` was that call's own `tool-end`. A turn that
 * ends without one - stopped mid-call, refused, killed by a provider error -
 * therefore left a card spinning forever, and every reader of that state
 * inherited the lie: the composer offered Stop for a turn that had finished,
 * and the conversation could not be told apart from one that was working.
 *
 * So the end of the turn settles them. Failed rather than done, because that
 * is what happened: the call did not produce a result. Anything that already
 * has output keeps it - a `tool-update` may have streamed most of a command's
 * output before the turn died, and that is the most useful thing on screen.
 */
function settleRunning(parts, why) {
  let touched = false;
  const settled = (parts ?? []).map((part) => {
    if (part?.type !== "tool" || part.state !== "running") return part;
    touched = true;
    return { ...part, state: "failed", output: part.output ?? why };
  });
  return touched ? settled : (parts ?? []);
}

/**
 * A thought, ended.
 *
 * How long the model spent thinking is worth saying, and it is only knowable
 * from the outside: the provider streams reasoning tokens and then simply
 * starts doing something else. So the moment it does - the first token of
 * prose, the first tool call, the end of the turn - closes the thought that
 * came before it.
 *
 * Recorded on the part, not held in the window, so it survives a reload and a
 * rejoin and reads the same in a transcript opened tomorrow. Both stamps are
 * needed rather than a duration, because a part is appended to as it grows and
 * the start would otherwise have to be recomputed from a length.
 */
function closeThought(parts, at) {
  const last = parts[parts.length - 1];
  if (last?.type !== "reasoning" || last.endedAt) return parts;
  return [...parts.slice(0, -1), { ...last, endedAt: at ?? Date.now() }];
}

export function applyEvent(message, event) {
  const parts = closeThought(message.parts ?? [], event.at);

  switch (event.type) {
    case "delta": {
      const last = parts[parts.length - 1];
      if (last?.type === "text") {
        return {
          ...message,
          parts: [...parts.slice(0, -1), { ...last, text: last.text + event.text }],
        };
      }
      return { ...message, parts: [...parts, { type: "text", text: event.text }] };
    }

    case "reasoning": {
      // Deliberately reads `message.parts` rather than the closed copy above:
      // a reasoning event must be able to extend the thought it belongs to,
      // and `closeThought` has just ended it. Every other case wants the
      // closed one, which is why the default is that way round.
      const live = message.parts ?? [];
      const last = live[live.length - 1];
      if (last?.type === "reasoning" && !last.endedAt) {
        return {
          ...message,
          parts: [...live.slice(0, -1), { ...last, text: last.text + event.text }],
        };
      }
      return {
        ...message,
        parts: [
          ...parts,
          { type: "reasoning", text: event.text, startedAt: event.at ?? Date.now() },
        ],
      };
    }

    case "tool-start": {
      const card = {
        type: "tool",
        callId: event.callId,
        name: event.name,
        title: event.title,
        args: event.args,
        state: "running",
      };
      // A call the message already holds is the same call again - a replayed
      // log landing on parts that were saved before the window went away -
      // and a second card for it would show the tool running twice.
      const held = parts.findIndex((part) => part.type === "tool" && part.callId === event.callId);
      if (held !== -1) {
        return {
          ...message,
          parts: [
            ...parts.slice(0, held),
            { ...parts[held], ...card, state: parts[held].state },
            ...parts.slice(held + 1),
          ],
        };
      }
      return { ...message, parts: [...parts, card] };
    }

    case "tool-update":
    case "tool-end": {
      const index = parts.findIndex((part) => part.type === "tool" && part.callId === event.callId);
      if (index === -1) return message;
      const part = parts[index];
      const next =
        event.type === "tool-update"
          ? {
              ...part,
              metadata: { ...part.metadata, ...event.metadata },
              title: event.title ?? part.title,
            }
          : {
              ...part,
              state: event.ok ? "done" : "failed",
              title: event.title ?? part.title,
              output: event.output,
              metadata: { ...part.metadata, ...event.metadata },
              durationMs: event.durationMs,
            };
      return { ...message, parts: [...parts.slice(0, index), next, ...parts.slice(index + 1)] };
    }

    /**
     * The person, mid-turn.
     *
     * A part of the reply rather than a message of its own, because that is
     * where it happened: under the tool card they were watching when they said
     * it, with the rest of the turn continuing beneath. A separate message in
     * the thread would have to sort itself after a reply that had not finished
     * being written, and would read as though they had waited politely.
     */
    case "steer":
      return { ...message, parts: [...parts, { type: "steer", text: event.text }] };

    /**
     * Something the turn survived, but that the person should be told about.
     *
     * A rate limit being waited out, or a long thread being compacted. Both
     * take real time and neither is an error, so a turn that silently pauses
     * for thirty seconds with nothing on screen is indistinguishable from one
     * that has hung. Merged into the previous notice when it repeats, because
     * "retrying in 1s / retrying in 2s / retrying in 4s" is one event with a
     * changing number rather than three things happening.
     */
    case "warning": {
      // Some warnings are only a `problems` map for the settings screen, with
      // nothing to say to the person mid-turn. Those stay invisible here.
      if (!event.message?.trim()) return message;
      const last = parts[parts.length - 1];
      if (last?.type === "notice" && last.kind === event.kind) {
        return {
          ...message,
          parts: [...parts.slice(0, -1), { ...last, text: event.message }],
        };
      }
      return {
        ...message,
        parts: [...parts, { type: "notice", kind: event.kind ?? "info", text: event.message }],
      };
    }

    /**
     * One of the user's hooks ran.
     *
     * Silent when it simply passed - a hook that approves every call would
     * otherwise put a line under every card - and a notice when it did
     * something the person should see: refused a call, sent the model back
     * to work, failed, or had a message for them.
     */
    case "hook": {
      const name = event.handler ?? event.event ?? "hook";
      const text =
        event.status === "blocked"
          ? `Hook "${name}" ${event.event === "Stop" || event.event === "SubagentStop" ? "sent the agent back to work" : `blocked ${event.subject ?? "the call"}`}${event.reason ? `: ${event.reason}` : "."}`
          : event.status === "stopped"
            ? `Hook "${name}" ended the turn${event.reason ? `: ${event.reason}` : "."}`
            : event.status === "timeout"
              ? `Hook "${name}" timed out and was ignored.`
              : event.status === "failed"
                ? `Hook "${name}" failed and was ignored${event.error ? `: ${event.error}` : "."}`
                : event.reason
                  ? `Hook "${name}": ${event.reason}`
                  : null;
      if (!text) return message;
      return { ...message, parts: [...parts, { type: "notice", kind: "hook", text }] };
    }

    /**
     * What the turn changed on disk, from the snapshot before its first
     * write to the folder at the end. On the message rather than in the
     * parts: it is a fact about the whole reply, drawn once under it, with
     * the revert that goes back to before the reply.
     */
    case "changes":
      return {
        ...message,
        changes: { from: event.from, to: event.to, cwd: event.cwd, files: event.files ?? [] },
      };

    case "error":
      return {
        ...message,
        parts: [
          ...settleRunning(parts, "The turn ended before this call finished."),
          { type: "error", text: event.message },
        ],
        state: "failed",
      };

    case "done":
      return {
        ...message,
        parts: settleRunning(
          parts,
          event.stopped === "cancelled"
            ? "The turn was stopped before this call finished."
            : "The turn ended before this call finished."
        ),
        state: event.stopped === "cancelled" ? "cancelled" : "done",
        usage: event.usage,
      };

    default:
      return message;
  }
}

/**
 * Is this conversation working?
 *
 * The composer's Stop button, the "typing" line and the timestamp under a
 * reply all turn on this one question, and it used to be answered by a single
 * field: whether some message still said `streaming`. That field is the
 * weakest of the three signals available and it is the one that drifts. What
 * a person actually watched was a tool card reading "Running" underneath a
 * reply the app had already timestamped as finished, over a permission card
 * asking whether that very call could go ahead.
 *
 * All three signals are asked instead. A running call is running whether or
 * not the message around it has settled; a question waiting on the person is
 * a tool suspended mid-execution, which is the most literally unfinished a
 * turn gets. `blocks` covers both shapes a transcript comes in: tool parts
 * inside a reply, which is what a live turn produces, and top-level tool
 * blocks, which is what seeded and imported threads hold.
 */
export function isThreadWorking(blocks, prompts = []) {
  if (prompts?.length) return true;
  return (blocks ?? []).some(
    (block) =>
      block?.status === "streaming" ||
      (block?.type === "tool" && block?.state === "running") ||
      (block?.parts ?? []).some((part) => part?.type === "tool" && part?.state === "running")
  );
}

/** The plain text of a message, for a preview line or a copy button. */
export function textOf(message) {
  return (message.parts ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("");
}

/**
 * What a reply's status becomes once its turn has ended.
 *
 * A reply that produced nothing and called no tool is a failure the user can
 * see, not an empty bubble they have to guess at. A turn that ran a tool and
 * said nothing afterwards did do something, and is shown as such.
 */
export function finishedStatus(message, event) {
  if (event.type === "error") return "error";
  if (event.type !== "done") return "streaming";
  const said = textOf(message).trim();
  const acted = (message.parts ?? []).some((part) => part.type === "tool");
  return said || acted ? "sent" : "error";
}

/**
 * The task list a conversation is working to, or an empty list.
 *
 * The list is not stored anywhere of its own: it is whatever the most recent
 * `todowrite` call wrote, and every call replaces the one before it. So the
 * current plan is found by walking the transcript backwards to the newest one,
 * which is also why this updates live - the same tool part the card renders is
 * the one read here, and it is patched in place as the turn runs.
 *
 * Both shapes of transcript entry are searched: a tool part inside a reply,
 * which is what a live turn produces, and a top-level tool block, which is what
 * seeded and imported threads hold.
 */
export function todosOf(messages) {
  const list = messages ?? [];
  for (let i = list.length - 1; i >= 0; i -= 1) {
    const message = list[i];
    if (!message) continue;
    if (message.type === "tool" && message.name === "todowrite") {
      const todos = message.metadata?.todos;
      if (Array.isArray(todos)) return todos;
    }
    const parts = message.parts ?? [];
    for (let p = parts.length - 1; p >= 0; p -= 1) {
      const part = parts[p];
      if (part?.type !== "tool" || part.name !== "todowrite") continue;
      const todos = part.metadata?.todos ?? part.args?.todos;
      if (Array.isArray(todos)) return todos;
    }
  }
  return [];
}

/**
 * The most recent plan a conversation was shown, and the call that produced it.
 *
 * The approval buttons belong on one card - the newest - and not on every plan
 * ever presented in the thread. Walking backwards is the same shape `todosOf`
 * uses, and for the same reason: the current answer is the last one written,
 * and both transcript shapes have to be searched because a live turn produces a
 * tool part inside a reply and a reloaded one produces a top-level block.
 */
export function latestPlan(messages) {
  const list = messages ?? [];
  for (let i = list.length - 1; i >= 0; i -= 1) {
    const message = list[i];
    if (!message) continue;
    if (message.type === "tool" && message.name === "present_plan") {
      const plan = message.metadata?.plan;
      if (plan) return { callId: message.callId ?? message.id, plan };
    }
    const parts = message.parts ?? [];
    for (let p = parts.length - 1; p >= 0; p -= 1) {
      const part = parts[p];
      if (part?.type !== "tool" || part.name !== "present_plan") continue;
      const plan = part.metadata?.plan;
      if (plan) return { callId: part.callId ?? `${message.id}:${p}`, plan };
    }
  }
  return null;
}
