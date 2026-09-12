import { call } from "./envelope";

/**
 * What a turn changed on disk, and the way back.
 *
 * The app takes a snapshot of the working folder the first time a turn is about
 * to write something, and compares against it when the turn ends. That is what
 * fills the strip under a reply: the files touched, how many lines each way,
 * the diff for any one of them, and an Undo that puts the folder back.
 *
 * Snapshots are content-addressed blobs in the workspace's cache folder rather
 * than a shadow git repository, because a working folder is very often not a
 * repository and git is not always installed.
 *
 * `cwd` travels with every call because a conversation can enter a worktree,
 * and the answer for one folder is not the answer for another.
 */
export function snapshotBridge() {
  return {
    /** Whether this folder can be snapshotted at all - some are too big. */
    available: (cwd) => call("snapshot_available", { cwd: String(cwd ?? "") }),

    /** The unified diff for one file of a turn, or for the whole turn. */
    diff: (options) => call("snapshot_diff", { options: options ?? {} }),

    /** Put the folder back to how it was before the turn. */
    revert: (options) => call("snapshot_revert", { options: options ?? {} }),
  };
}
