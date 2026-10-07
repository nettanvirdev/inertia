import * as React from "react";
import { useWorkspace } from "@/lib/workspace";

/**
 * The workspace's own working folder, and the ones chosen recently.
 *
 * Two different things kept in one place because they live in one file. The
 * DEFAULT is what an agent uses when it has not been pointed anywhere; the
 * RECENTS are a convenience, because moving between two or three projects is
 * the normal case and re-picking a folder through a native dialog every time is
 * the kind of friction that stops people using the feature at all.
 *
 * Both are read from `settings.app` rather than from local storage, so they
 * belong to the workspace and travel with it.
 */

/** Enough to cover the projects anyone is actually switching between. */
const MAX_RECENT = 6;

/**
 * The last segment of a path, for a label.
 *
 * Both separators, which is not a nicety here: this app is Windows first, its
 * paths look like `D:\projects\inertia`, and splitting on the forward slash
 * alone returned the whole path unchanged. So the working-folder chip in the
 * composer, whose entire job is to show a short name, was showing the full
 * path instead.
 */
export function basename(dir) {
  const parts = String(dir ?? "")
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/);
  return parts[parts.length - 1] || String(dir ?? "");
}

export function useWorkingFolder() {
  const { client, configured } = useWorkspace();
  const [folder, setFolder] = React.useState(null);
  const [recents, setRecents] = React.useState([]);

  const load = React.useCallback(async () => {
    if (!configured) return;
    try {
      const doc = (await client.readDocument("settings.app", {})) ?? {};
      setFolder(doc.workingDirectory || null);
      setRecents(Array.isArray(doc.recentWorkingFolders) ? doc.recentWorkingFolders : []);
    } catch {
      // A settings file that cannot be read is the settings pane's problem to
      // report; the chip falls back to showing the default and still works.
    }
  }, [client, configured]);

  React.useEffect(() => {
    load();
  }, [load]);

  const remember = React.useCallback(
    async (dir) => {
      if (!dir || !configured) return;
      // Optimistic, because the menu is open and the user is looking at it.
      setRecents((prev) => [dir, ...prev.filter((one) => one !== dir)].slice(0, MAX_RECENT));
      try {
        const doc = (await client.readDocument("settings.app", {})) ?? {};
        const previous = Array.isArray(doc.recentWorkingFolders) ? doc.recentWorkingFolders : [];
        await client.writeDocument("settings.app", {
          ...doc,
          recentWorkingFolders: [dir, ...previous.filter((one) => one !== dir)].slice(
            0,
            MAX_RECENT
          ),
        });
      } catch {
        // Losing a recent entry is not worth a message. The folder itself was
        // already saved on the agent by the caller.
      }
    },
    [client, configured]
  );

  return { folder, recents, remember, reload: load };
}
