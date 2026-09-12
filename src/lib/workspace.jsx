import * as React from "react";
import { workspaceClient, isNativeWorkspace } from "./workspace-client";

/**
 * Workspace state for the whole app.
 *
 * Two things live here and nothing else does. `status` answers the question the
 * shell asks before it renders anything - is there a folder yet - and
 * `useCollection` is the hook every managing screen uses to read and write one
 * kind of record.
 *
 * `useCollection` deliberately owns the whole read/write cycle rather than
 * handing back a setter: a save has to hit the disk, and a list that updated
 * optimistically and then failed would be a UI claiming something that is not
 * on disk. So writes await the round trip and reload from the source.
 */

const WorkspaceContext = React.createContext(null);

const IDLE = { phase: "loading", status: null, error: null };

export function WorkspaceProvider({ children }) {
  const client = React.useMemo(() => workspaceClient(), []);
  const [state, setState] = React.useState(IDLE);

  const refresh = React.useCallback(async () => {
    try {
      const status = await client.status();
      setState({ phase: status.configured ? "ready" : "setup", status, error: null });
      return status;
    } catch (error) {
      setState({ phase: "error", status: null, error: error.message });
      return null;
    }
  }, [client]);

  React.useEffect(() => {
    refresh();
  }, [refresh]);

  // The folder can be configured, moved or reset from somewhere this provider
  // did not initiate - another window, a settings pane in a different tree.
  // Status is the one piece of state that decides whether the app renders at
  // all, so it listens rather than assuming it is the only writer.
  React.useEffect(() => {
    if (typeof client.onChanged !== "function") return undefined;
    return client.onChanged((payload) => {
      // A record or a settings document changed, not the folder itself. Only a
      // payload naming neither is news about where the workspace is.
      if (payload && (payload.collection || payload.document)) return;
      refresh();
    });
  }, [client, refresh]);

  const configure = React.useCallback(
    async (dir) => {
      const result = await client.configure(dir);
      setState({ phase: "ready", status: result, error: null });
      return result;
    },
    [client]
  );

  const reset = React.useCallback(async () => {
    const result = await client.reset();
    setState({ phase: "setup", status: result, error: null });
    return result;
  }, [client]);

  const value = React.useMemo(
    () => ({
      ...state,
      client,
      native: isNativeWorkspace(),
      root: state.status?.root ?? null,
      configured: Boolean(state.status?.configured),
      refresh,
      configure,
      reset,
      browse: client.browse,
      chooseFolder: client.chooseFolder,
      inspect: client.inspect,
      reveal: client.reveal,
      listDir: client.listDir,
      readBytes: client.readBytes,
      writeBytes: client.writeBytes,
      removeFile: client.removeFile,
      wipe: client.wipe,
    }),
    [state, client, refresh, configure, reset]
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace() {
  const value = React.useContext(WorkspaceContext);
  if (!value) throw new Error("useWorkspace must be used inside a WorkspaceProvider");
  return value;
}

/**
 * One collection, read from disk and written back to it.
 *
 * `enabled` exists so a tab that is not on screen does not read the disk; the
 * integrations view mounts four of these and only one is ever visible.
 */
export function useCollection(name, { enabled = true } = {}) {
  const { client, configured } = useWorkspace();
  const [items, setItems] = React.useState([]);
  const [loading, setLoading] = React.useState(enabled);
  const [error, setError] = React.useState(null);

  const live = enabled && configured;

  const reload = React.useCallback(async () => {
    if (!live) {
      setItems([]);
      setLoading(false);
      return [];
    }
    setLoading(true);
    try {
      const rows = await client.list(name);
      setItems(rows);
      setError(null);
      return rows;
    } catch (failure) {
      setError(failure.message);
      return [];
    } finally {
      setLoading(false);
    }
  }, [client, name, live]);

  React.useEffect(() => {
    reload();
  }, [reload]);

  // A write from another pane - or another window - should not leave this list
  // showing yesterday's answer.
  React.useEffect(() => {
    if (!live || typeof client.onChanged !== "function") return undefined;
    return client.onChanged((payload) => {
      if (!payload || payload.memory || payload.collection === name) reload();
    });
  }, [client, name, live, reload]);

  const save = React.useCallback(
    async (record) => {
      const saved = await client.put(name, record);
      await reload();
      return saved;
    },
    [client, name, reload]
  );

  const patch = React.useCallback(
    async (id, changes) => {
      const saved = await client.patch(name, id, changes);
      await reload();
      return saved;
    },
    [client, name, reload]
  );

  const remove = React.useCallback(
    async (id) => {
      await client.remove(name, id);
      await reload();
    },
    [client, name, reload]
  );

  return { items, loading, error, reload, save, patch, remove };
}
