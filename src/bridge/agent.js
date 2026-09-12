import { call, subscribe } from "./envelope";

/**
 * The agent bridge.
 *
 * A turn is started, an id comes back, and everything that happens arrives on
 * one event stream tagged with that id: tool calls starting and finishing,
 * partial output from a command still running, and the two things that suspend
 * a tool mid-call while a person decides.
 *
 * Those two - permission and question - arrive on the same channel tagged with
 * a `channel` field rather than on channels of their own. One subscription
 * means a window cannot end up listening for answers and missing questions,
 * which is the worst failure this feature has: a tool blocked forever with
 * nothing on screen to say why.
 *
 * Every method the preload declared is present, because callers reach them
 * through wrappers that do not all guard. The ones with no backend answer a
 * refusal in the envelope the caller already unwraps - a sentence it can show -
 * rather than being absent and surfacing as `is not a function` several frames
 * away from the cause.
 */
const NOT_YET = (what) =>
  `${what} is not wired up in this build yet. The conversation itself works; this is a part of the turn record that has no backend.`;

export function agentBridge() {
  return {
    run: (request) => call("agent_run", { request: request ?? {} }),
    cancel: (id) => call("agent_cancel", { id }),
    cancelAll: () => call("agent_cancel_all"),
    steer: (id, text) => call("agent_steer", { id, text: String(text ?? "") }),
    forget: (sessionId) => call("agent_forget", { sessionId: String(sessionId ?? "") }),

    /** What survived a reload, so a quiet turn can be told from a dead one. */
    active: () => call("agent_active"),

    rules: (agentId) => call("agent_rules", { agentId: agentId ?? null }),
    tools: () => call("tools_list"),
    invalidate: () => call("tools_invalidate"),
    openPath: (target) => call("agent_open_path", { target: String(target ?? "") }),

    reply: (id, answer, message) =>
      call("permission_reply", { id, answer: String(answer ?? "reject"), message: message ?? "" }),
    waiting: (sessionId) => call("permission_waiting", { sessionId: sessionId ?? null }),
    grants: () => call("permission_grants"),
    revoke: (sessionId, tool, pattern) =>
      call("permission_revoke", { sessionId: sessionId ?? null, tool, pattern: pattern ?? null }),

    /*
     * Every turn is written down as it happens - the same events that went to
     * this window - so a reload has something to draw and a conversation from
     * last week can be read back.
     */
    record: (id) => call("agent_record", { id: String(id ?? "") }),
    history: (threadId) => call("agent_history", { threadId: threadId ?? null }),
    /** One summarising call, no tools, no turn. What `/compact` is made of. */
    summarize: (request) => call("agent_summarize", { request: request ?? {} }),

    // The other thing that suspends a tool: the model asking the person a
    // question. Same mechanism as an approval card, on the same channel.
    questions: (sessionId) => call("agent_questions_waiting", { sessionId: sessionId ?? null }),
    answer: (id, value) => call("agent_answer", { id, value: String(value ?? "") }),
    dismiss: (id) => call("agent_dismiss", { id }),

    onEvent: (callback) => subscribe("agent:event", callback),
  };
}
