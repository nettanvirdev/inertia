import * as React from "react";
import {
  ChevronRight,
  Folder,
  FolderOpen,
  FolderTree,
  HardDrive,
  Package,
  TriangleAlert,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useWorkspace } from "@/lib/workspace";
import { useDateFormat } from "@/lib/datetime";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { CopyButton } from "@/components/ui/copy-button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip } from "@/components/ui/tooltip";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";
import { WorkingFolder } from "./WorkingFolder";

const UNITS = ["B", "KB", "MB", "GB", "TB"];

function formatBytes(bytes) {
  const total = Number(bytes) || 0;
  if (total < 1) return "empty";
  let value = total;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // whole numbers below a kilobyte, and once past 10 of any unit the decimal
  // stops carrying information anyone reads
  const rounded = unit === 0 || value >= 10 ? Math.round(value) : value.toFixed(1);
  return `${rounded} ${UNITS[unit]}`;
}

/**
 * Nest the flat `client.tree()` list by path prefix. The list arrives in whatever
 * order the walker produced, so every node is registered before any parent is
 * looked up. A folder whose ancestors are all absent stands at the root rather
 * than having a parent invented for it, because the list is the only truth about
 * what actually exists on disk.
 */
function buildFolderTree(directories) {
  const byPath = new Map();
  for (const dir of directories) byPath.set(dir.path, { dir, children: [] });

  const roots = [];
  for (const node of byPath.values()) {
    let cut = node.dir.path.lastIndexOf("/");
    let parent = null;
    // climb past ancestors missing from the list so a gap does not orphan a
    // whole subtree at the root
    while (cut > 0 && !parent) {
      const candidate = node.dir.path.slice(0, cut);
      parent = byPath.get(candidate) ?? null;
      cut = candidate.lastIndexOf("/");
    }
    if (parent) parent.children.push(node);
    else roots.push(node);
  }
  return roots;
}

/** Every path that should start closed: anything nested that has children of its own. */
function deepBranchPaths(nodes, depth = 0, out = new Set()) {
  for (const node of nodes) {
    if (depth > 0 && node.children.length) out.add(node.dir.path);
    deepBranchPaths(node.children, depth + 1, out);
  }
  return out;
}

/** One directory from `client.tree()`, plus its subtree when it is open. */
function FolderRow({ node, depth, collapsed, onToggle, native, onReveal }) {
  const dir = node.dir;
  const hasChildren = node.children.length > 0;
  const expanded = hasChildren && !collapsed.has(dir.path);
  // an open folder is drawn open. The chevron says the same thing, but the
  // glyph is what the eye reads first when scanning a tree.
  const Glyph = expanded ? FolderOpen : Folder;

  return (
    <>
      <div className="flex items-center" style={{ paddingLeft: `${depth * 0.875}rem` }}>
        {/* the slot is reserved whether or not a chevron lives in it, so labels
            line up with their siblings instead of drifting by a leaf */}
        <span className="flex size-4 shrink-0 items-center justify-center">
          {hasChildren ? (
            <button
              type="button"
              aria-label={`${expanded ? "Collapse" : "Expand"} ${dir.label}`}
              aria-expanded={expanded}
              // the chevron sits on top of the reveal row, so the toggle must not
              // also throw open a file explorer window
              onClick={(event) => {
                event.stopPropagation();
                onToggle(dir.path);
              }}
              className={cn(
                "flex size-4 items-center justify-center rounded text-muted-foreground outline-none",
                "transition-colors duration-150 ease-out hover:text-foreground",
                "focus-visible:fill-control-hover"
              )}
            >
              <ChevronRight
                className={cn(
                  "size-3 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
                  expanded && "rotate-90"
                )}
                aria-hidden="true"
              />
            </button>
          ) : null}
        </span>
        <button
          type="button"
          disabled={!native}
          onClick={() => onReveal(dir.path)}
          className={cn(
            "flex min-w-0 flex-1 items-center gap-2.5 rounded-lg py-1.5 pr-2 pl-1.5 text-left outline-none",
            "transition-colors duration-150 ease-out",
            "focus-visible:fill-control-hover",
            native ? "hover:fill-control-hover" : "cursor-default"
          )}
        >
          <Glyph
            className={cn(
              "size-3.5 shrink-0",
              dir.exists ? "text-muted-foreground" : "text-muted-foreground/50"
            )}
            aria-hidden="true"
          />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-xs text-foreground/90">{dir.label}</span>
            <span className="block truncate text-[0.6875rem] leading-tight text-muted-foreground">
              {dir.description}
            </span>
          </span>
          <span className="w-16 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
            {dir.entries === 1 ? "1 item" : `${dir.entries} items`}
          </span>
          <span className="w-16 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
            {formatBytes(dir.bytes)}
          </span>
        </button>
      </div>
      {/* The subtree folds as one block; its rows keep the card's rhythm. */}
      <Collapse open={expanded} innerClassName="flex flex-col gap-0.5">
        {node.children.map((child) => (
          <FolderRow
            key={child.dir.path}
            node={child}
            depth={depth + 1}
            collapsed={collapsed}
            onToggle={onToggle}
            native={native}
            onReveal={onReveal}
          />
        ))}
      </Collapse>
    </>
  );
}

export function WorkspacePane() {
  const { status, root, configured, native, client, browse, inspect, configure, refresh } =
    useWorkspace();
  const { toast } = useToast();
  // The manifest date is written out in the user's own format - see lib/datetime.
  const { formatDate } = useDateFormat();

  const [tree, setTree] = React.useState(null);
  const [loadingTree, setLoadingTree] = React.useState(false);
  const [treeError, setTreeError] = React.useState(null);
  const [pending, setPending] = React.useState(null); // an inspect report awaiting confirmation
  const [switching, setSwitching] = React.useState(false);

  const manifest = status?.manifest ?? null;
  // Three different wrong states, and they need different words. `ahead` is
  // the serious one: the folder is fine, this app is the problem, and picking
  // a different folder is not the fix.
  const broken = Boolean(status?.missing || status?.stale || status?.ahead);

  // The tree walks the disk, so it is loaded on its own rather than blocking the
  // path and the manifest facts, which are already in hand.
  const loadTree = React.useCallback(async () => {
    if (!configured) {
      setTree(null);
      return;
    }
    setLoadingTree(true);
    try {
      setTree(await client.tree());
      setTreeError(null);
    } catch (failure) {
      setTree(null);
      setTreeError(failure.message);
    } finally {
      setLoadingTree(false);
    }
  }, [client, configured]);

  React.useEffect(() => {
    loadTree();
  }, [loadTree, root]);

  const folderRoots = React.useMemo(
    () => buildFolderTree(tree?.directories ?? []),
    [tree]
  );

  const [collapsed, setCollapsed] = React.useState(() => new Set());

  // Deeper folders start closed so the list opens at a size someone can read.
  // Re-reading the tree resets this, which is also the moment its shape can have
  // changed underneath any paths that were remembered.
  React.useEffect(() => {
    setCollapsed(deepBranchPaths(folderRoots));
  }, [folderRoots]);

  const toggleFolder = React.useCallback((path) => {
    setCollapsed((current) => {
      const next = new Set(current);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }, []);

  const totalBytes = React.useMemo(() => {
    // DIRECTORIES lists nested folders too, so only the top level is summed or
    // plugins/mcp would be counted inside plugins and again on its own.
    if (!tree?.directories) return 0;
    return tree.directories
      .filter((dir) => !dir.path.includes("/"))
      .reduce((sum, dir) => sum + (Number(dir.bytes) || 0), 0);
  }, [tree]);

  async function revealPath(relPath) {
    if (!native) return;
    try {
      await client.reveal(relPath);
    } catch (failure) {
      toast({ title: "Could not open that folder", description: failure.message, variant: "danger" });
    }
  }

  async function pickFolder() {
    try {
      const dir = await browse();
      if (!dir) return; // cancelled, which is not a failure
      const report = await inspect(dir);
      if (report.action === "blocked") {
        toast({
          title: "That folder cannot be used",
          description: report.error ?? "Pick another folder.",
          variant: "danger",
        });
        return;
      }
      setPending(report);
    } catch (failure) {
      toast({ title: "Could not read that folder", description: failure.message, variant: "danger" });
    }
  }

  async function applyFolder() {
    if (!pending) return;
    setSwitching(true);
    try {
      const next = await configure(pending.path);
      setPending(null);
      toast({
        title: next.adopted ? "Workspace adopted" : "Workspace created",
        description: pending.path,
        variant: "success",
      });
      await refresh();
    } catch (failure) {
      toast({ title: "Could not switch folder", description: failure.message, variant: "danger" });
    } finally {
      setSwitching(false);
    }
  }

  if (!configured) {
    return (
      <div className="w-full">
        <EmptyState
          icon={FolderTree}
          title="No workspace folder yet"
          description="Inertia keeps everything it knows in one folder on your disk. Choose where it lives and the app fills it in."
          action={
            native ? (
              <Button variant="subtle" size="xs" onClick={pickFolder}>
                Choose a folder
              </Button>
            ) : null
          }
        />
        {!native ? (
          <p className="mx-auto max-w-[46ch] text-center text-[0.6875rem] leading-relaxed text-muted-foreground">
            This is a browser preview, so there is no folder to pick. Run the desktop app to set
            one up.
          </p>
        ) : null}

        <ConfirmDialog
          open={Boolean(pending)}
          onOpenChange={(open) => !open && setPending(null)}
          title={pending?.action === "adopt" ? "Use this workspace?" : "Create a workspace here?"}
          description={
            pending?.action === "adopt"
              ? `${pending?.path} already holds an Inertia workspace. Its settings, agents and threads become the ones this app shows.`
              : `Inertia will write its folder structure into ${pending?.path}. Anything already in that folder is left alone.`
          }
          confirmLabel={pending?.action === "adopt" ? "Use this folder" : "Create workspace"}
          loading={switching}
          onConfirm={applyFolder}
        />
      </div>
    );
  }

  return (
    <div className="w-full">
      {broken ? (
        <SettingsSection flat>
          <SettingsCard className="flex items-start gap-2.5">
            <TriangleAlert className="mt-px size-3.5 shrink-0 text-warning-ink" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <p className="text-xs text-foreground/90">
                {status.ahead
                  ? "This folder was made by a newer version of Inertia."
                  : status.missing
                    ? "This folder is no longer there."
                    : "This folder is no longer an Inertia workspace."}
              </p>
              <p className="mt-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                {status.ahead
                  ? `It uses workspace format ${status.formatVersion} and this app understands ${status.appFormatVersion}. Writing to it with the older rules could lose data, so update Inertia rather than picking a different folder.`
                  : status.missing
                    ? "It may have been moved, renamed, or it lives on a drive that is not mounted. Nothing is written until it is back or you pick another folder."
                    : "Its manifest is gone, so the app cannot tell it apart from an ordinary folder. Point Inertia at the right copy, or pick this one again to rebuild the structure in place."}
              </p>
            </div>
            {/* Picking another folder is the answer to two of these three and
                the wrong instinct for the third, so it is not offered there. */}
            {status.ahead ? null : (
              <Button variant="subtle" size="xs" disabled={!native} onClick={pickFolder}>
                Pick a folder
              </Button>
            )}
          </SettingsCard>
        </SettingsSection>
      ) : null}

      <SettingsSection flat
        title="Location"
        description="One folder holds settings, agents, skills, plugins, conversations, history, memory and secrets."
      >
        <SettingsCard className="flex flex-col gap-2.5">
          <div className="flex items-center gap-2">
            <HardDrive className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <p className="min-w-0 flex-1 truncate font-mono text-xs text-foreground" title={root}>
              {root}
            </p>
            <CopyButton value={root} label="Copy path" />
            <Tooltip
              content={native ? "Show in Explorer" : "Only available in the desktop app"}
            >
              <span>
                <Button
                  variant="subtle"
                  size="xs"
                  disabled={!native}
                  onClick={() => revealPath("")}
                >
                  <FolderOpen />
                  Show in Explorer
                </Button>
              </span>
            </Tooltip>
          </div>

          {/* the facts that identify this workspace, kept deliberately quiet */}
          <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
            {[
              manifest?.createdAt ? `Created ${formatDate(manifest.createdAt)}` : null,
              manifest?.version != null ? `Format v${manifest.version}` : null,
              manifest?.id ? `ID ${manifest.id}` : null,
            ]
              .filter(Boolean)
              .join(" · ") || "No manifest could be read from this folder."}
          </p>
        </SettingsCard>
      </SettingsSection>

      <SettingsSection flat
        title="What is in it"
        description="Every folder the workspace owns. Selecting one opens it in your file explorer."
      >
        <SettingsCard className="flex flex-col gap-0.5 p-1.5">
          {loadingTree ? (
            <div role="status" aria-busy="true" aria-label="Reading the workspace" className="flex flex-col gap-2 p-1.5">
              {Array.from({ length: 6 }).map((_, i) => (
                <Skeleton key={i} className="h-7 w-full" />
              ))}
            </div>
          ) : treeError ? (
            <p className="p-2 text-[0.6875rem] leading-relaxed text-muted-foreground">
              {treeError}
            </p>
          ) : tree?.directories?.length ? (
            <>
              {folderRoots.map((node) => (
                <FolderRow
                  key={node.dir.path}
                  node={node}
                  depth={0}
                  collapsed={collapsed}
                  onToggle={toggleFolder}
                  native={native}
                  onReveal={revealPath}
                />
              ))}
              <div className="mt-1 flex items-center gap-2.5 px-2 pt-1.5 pb-0.5">
                <Package className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                <span className="min-w-0 flex-1 text-xs text-foreground/90">Total on disk</span>
                <span className="text-[0.6875rem] tabular-nums text-muted-foreground">
                  {formatBytes(totalBytes)}
                </span>
              </div>
            </>
          ) : (
            <p className="p-2 text-[0.6875rem] leading-relaxed text-muted-foreground">
              Nothing to measure yet.
            </p>
          )}
        </SettingsCard>
      </SettingsSection>

      <WorkingFolder />

      <SettingsSection title="Change folder">
        <SettingsRow
          label="Move to another folder"
          description={
            native
              ? "Point Inertia at a different folder, either an empty one to start fresh or an existing workspace to pick up where it left off. The folder you are using now is left exactly as it is."
              : "The folder picker needs the desktop app. In a browser preview there is no disk to point at."
          }
          control={
            <Button variant="subtle" size="xs" disabled={!native} onClick={pickFolder}>
              <FolderTree />
              Change folder
            </Button>
          }
        />
      </SettingsSection>

      <ConfirmDialog
        open={Boolean(pending)}
        onOpenChange={(open) => !open && setPending(null)}
        title={pending?.action === "adopt" ? "Switch to this workspace?" : "Create a workspace here?"}
        description={
          pending?.action === "adopt"
            ? `${pending?.path} already holds an Inertia workspace. Everything on screen is replaced by what that folder contains: its settings, agents, skills, plugins, threads and secrets. Your current folder at ${root} is left untouched, not deleted.`
            : `Inertia will write its folder structure into ${pending?.path} and start empty there. Everything on screen is replaced by that new workspace. Your current folder at ${root} is left untouched, not deleted, so you can switch back to it at any time.`
        }
        confirmLabel={pending?.action === "adopt" ? "Switch workspace" : "Create workspace"}
        loading={switching}
        onConfirm={applyFolder}
      />
    </div>
  );
}
