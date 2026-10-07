import * as React from "react";

/**
 * The folder a Run button runs in.
 *
 * A code block has no idea which conversation it is in, and the answer is a
 * property of the conversation: the folder this agent works in, or the worktree
 * the thread entered. The chat view knows both and puts the answer here, so the
 * block stays a block. Null means "wherever the workspace is", which is what
 * the backend falls back to.
 */
export const RunFolder = React.createContext(null);
