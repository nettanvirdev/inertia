import { call } from "./envelope";

/**
 * The memory bridge: what an agent knows before the conversation starts.
 *
 * The Memory *screen* does not come through here. It reads and writes the
 * `memory` collection over the workspace bridge, the same way it reads agents
 * and routines, and the backend routes that one collection across both places a
 * memory can live. Keeping the screen on the generic path is what lets it stay
 * the renderer that was ported, unchanged.
 *
 * What is here is the part a collection cannot express: searching, telling the
 * backend which project is in front of the person, and running a capture pass
 * on demand rather than waiting for a conversation to go quiet.
 */
export function memoryBridge() {
  return {
    /** Settings, plus where both stores actually are on disk. */
    status: () => call("memory_status"),

    /**
     * Which folder the person is working in.
     *
     * Memory is scoped by it, and the screen asks for a collection by name with
     * nowhere to put a folder - so the backend is told separately.
     */
    setProject: (cwd) => call("memory_set_project", { cwd: cwd ?? null }),

    /** Full text of the memories a query is about. Finding nothing is an answer. */
    recall: (query, limit) => call("memory_recall", { query: query ?? "", limit: limit ?? null }),

    save: (record) => call("memory_save", { record: record ?? {} }),
    forget: (id) => call("memory_forget", { id }),

    /** Look for something worth keeping now, rather than at the next quiet moment. */
    captureNow: (threadId) => call("memory_capture_now", { threadId }),
  };
}
