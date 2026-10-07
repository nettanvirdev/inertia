import * as React from "react";
import { permission, question, watchPrompts } from "@/lib/agent";

/**
 * The questions a running turn is waiting on.
 *
 * Two different things arrive here and they are deliberately kept in one
 * ordered list: a permission ask, which suspends a tool that is about to do
 * something, and a question the model asked, which suspends a tool that wants
 * an answer. They queue together because from the user's point of view they are
 * the same interruption, and showing a permission card above a question card
 * that arrived first would misrepresent what happened.
 *
 * The subscription is a single one for both kinds. A window that listened for
 * one and not the other would hang a tool call indefinitely with no sign of
 * why, which is the worst failure this feature has.
 */
/** The conversation a session id belongs to. `parent/task-3` is parent's. */
function rootSession(sessionId) {
  return String(sessionId ?? "").split("/")[0];
}

export function usePrompts(threadId) {
  const [prompts, setPrompts] = React.useState([]);

  React.useEffect(() => {
    // Anything already waiting when this mounts - the user switched threads
    // mid-turn, or reopened a window - has to be recovered, or the tool that is
    // blocked on it never gets an answer. Both kinds, because both suspend a
    // call: recovering only permissions leaves a model's own question waiting
    // on a card that no longer exists, which is the one prompt the person
    // definitely meant to answer.
    //
    // Permissions first, then questions, rather than interleaved by time: a
    // permission ask carries no timestamp, so the order they originally
    // arrived in is not recoverable and inventing one would be a guess.
    let alive = true;
    Promise.all([permission.waiting(threadId), question.waiting(threadId)])
      .then(([asks, questions]) => {
        if (!alive) return;
        setPrompts([
          ...asks.map((q) => ({ channel: "permission", question: q })),
          ...questions.map((q) => ({ channel: "question", question: q })),
        ]);
      })
      .catch(() => {});

    const off = watchPrompts((event) => {
      if (event.type === "settled") {
        setPrompts((prev) => prev.filter((p) => p.question.id !== event.id));
        return;
      }
      if (event.type !== "asked") return;

      // A question raised by another thread's turn belongs in that thread. A
      // subagent runs as `parent/task-3`, and there is only one person to ask,
      // so its questions belong here - matching the whole id would silently
      // drop them and leave the subagent's tool blocked until cancel.
      if (
        threadId &&
        event.question.sessionId &&
        rootSession(event.question.sessionId) !== threadId
      ) {
        return;
      }

      setPrompts((prev) =>
        prev.some((p) => p.question.id === event.question.id)
          ? prev
          : [...prev, { channel: event.channel, question: event.question }]
      );
    });

    return () => {
      alive = false;
      off?.();
    };
  }, [threadId]);

  const dismiss = React.useCallback((id) => {
    setPrompts((prev) => prev.filter((p) => p.question.id !== id));
  }, []);

  const replyPermission = React.useCallback(
    (id, answer, message) => {
      // Removed optimistically. The backend has already been told, and a
      // card that lingers after the click reads as the click not working.
      dismiss(id);
      permission.reply(id, answer, message)?.catch(() => {});
    },
    [dismiss]
  );

  const answerQuestion = React.useCallback(
    (id, value) => {
      dismiss(id);
      question.answer(id, value)?.catch(() => {});
    },
    [dismiss]
  );

  const dismissQuestion = React.useCallback(
    (id) => {
      dismiss(id);
      question.dismiss(id)?.catch(() => {});
    },
    [dismiss]
  );

  return { prompts, replyPermission, answerQuestion, dismissQuestion };
}
