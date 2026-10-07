import * as React from "react";
import { useWorkspace } from "@/lib/workspace";

/**
 * The user's photo, as a file in the workspace.
 *
 * It used to be a data URL wedged into settings/identity.json. That worked, and
 * it was wrong for the same reason the whole workspace is a folder of readable
 * files: a photo pasted into JSON as forty thousand characters of base64 is not
 * something a person can look at, replace, or copy to another machine. Now it
 * is `settings/avatar.png`, and replacing it is dropping a file in a folder.
 *
 * The document keeps the PATH and a version stamp. The bytes are read back on
 * demand and turned into a data URL, because the window's Content-Security
 * Policy allows `data:` images and nothing else local.
 */

/** Where it goes. One file, overwritten, so a folder never fills with avatars. */
export const AVATAR_BASENAME = "settings/avatar";

/**
 * Where an agent's picture goes.
 *
 * The same idea, one folder along: `agents/pictures/<agent id>.png`. Teammates
 * get faces for the same reason the user does - eight agents that differ only
 * by two letters in a circle are eight agents nobody can tell apart at a
 * glance - and it is a file for the same reason too, so it copies with the
 * folder and can be replaced without opening the app.
 *
 * The agent record keeps `avatarFile` and `avatarUpdatedAt`, exactly as the
 * profile does. `inertia_set_picture` writes the same two fields, so a picture
 * an agent was given by another agent is the same thing as one dropped in here.
 */
export const AGENT_PICTURE_DIR = "agents/pictures";

export function agentPathFor(agentId, mime) {
  return `${AGENT_PICTURE_DIR}/${agentId}.${EXTENSIONS[mime] ?? "png"}`;
}

const EXTENSIONS = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
};

const MIMES = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
};

/** `data:image/png;base64,AAA...` into its two useful halves, or null. */
export function splitDataUrl(dataUrl) {
  const match = /^data:([^;,]+);base64,(.*)$/s.exec(String(dataUrl ?? ""));
  if (!match) return null;
  return { mime: match[1], base64: match[2] };
}

export function pathFor(mime) {
  return `${AVATAR_BASENAME}.${EXTENSIONS[mime] ?? "png"}`;
}

function mimeFor(relPath) {
  const ext = String(relPath ?? "")
    .split(".")
    .pop()
    ?.toLowerCase();
  return MIMES[ext] ?? "image/png";
}

/**
 * The same, for an agent.
 *
 * Returns the patch to merge into the agent record. The old file is removed
 * first when the type changed, or an agent that went from a JPEG to a PNG
 * would leave the JPEG behind for the next reader to wonder about.
 */
export async function saveAgentPicture(workspace, agentId, dataUrl, previousFile) {
  const parts = splitDataUrl(dataUrl);
  if (!parts) return { avatarFile: null, avatarUrl: null, avatarUpdatedAt: Date.now() };

  if (!workspace?.native || !workspace?.configured) {
    return { avatarFile: null, avatarUrl: dataUrl, avatarUpdatedAt: Date.now() };
  }

  const relPath = agentPathFor(agentId, parts.mime);
  if (previousFile && previousFile !== relPath) {
    await workspace.removeFile(previousFile).catch(() => {});
  }
  await workspace.writeBytes(relPath, parts.base64);
  return { avatarFile: relPath, avatarUrl: null, avatarUpdatedAt: Date.now() };
}

/** Remove an agent's picture, file and reference both. */
export async function clearAgentPicture(workspace, agent) {
  if (agent?.avatarFile && workspace?.native && workspace?.configured) {
    await workspace.removeFile(agent.avatarFile).catch(() => {});
  }
  return { avatarFile: null, avatarUrl: null, avatarUpdatedAt: Date.now() };
}

/**
 * Write the picked photo into the workspace, and say where it went.
 *
 * Returns the patch to merge into the profile. Outside the desktop app there is no
 * folder, so the data URL is kept as it was - a browser preview should still
 * show the photo it was just given rather than silently dropping it.
 */
export async function saveAvatar(workspace, dataUrl) {
  const parts = splitDataUrl(dataUrl);
  if (!parts) return { avatarFile: null, avatarUrl: null, avatarUpdatedAt: Date.now() };

  if (!workspace?.native || !workspace?.configured) {
    return { avatarFile: null, avatarUrl: dataUrl, avatarUpdatedAt: Date.now() };
  }

  const relPath = pathFor(parts.mime);
  await workspace.writeBytes(relPath, parts.base64);
  // The data URL is dropped once the file exists, so the document does not
  // carry the same image twice and the file is the only copy that matters.
  return { avatarFile: relPath, avatarUrl: null, avatarUpdatedAt: Date.now() };
}

/** Remove the file as well as the reference. A cleared photo should be gone. */
export async function clearAvatar(workspace, user) {
  if (user?.avatarFile && workspace?.native && workspace?.configured) {
    await workspace.removeFile(user.avatarFile).catch(() => {});
  }
  return { avatarFile: null, avatarUrl: null, avatarUpdatedAt: Date.now() };
}

/**
 * The photo as something an `img` can show.
 *
 * Re-reads when the path or the version stamp changes, which covers both "the
 * user picked a new photo" and "the user replaced the file in the folder and
 * saved the profile". It does not watch the file, because a poll on every
 * render for an image that changes once a year is not a trade worth making.
 */
/**
 * Pictures already read, by file and version.
 *
 * The user's photo is read once on a settings screen. An agent's is read by
 * every avatar on screen - the sidebar, the thread list, every message bubble,
 * the agents grid - and without this each one of those is its own trip across
 * the IPC bridge for the same few kilobytes, repeated on every mount. Keyed by
 * the version stamp as well as the path, so replacing a picture invalidates it
 * rather than pinning the old one forever.
 */
const settled = new Map();

/** Bounded, because a workspace can hold more agents than anyone will look at. */
const MAX_CACHED = 64;

function remember(key, value) {
  if (settled.size >= MAX_CACHED) settled.delete(settled.keys().next().value);
  settled.set(key, value);
}

/**
 * The picture, if it is already known - no hook, no read.
 *
 * For the places that draw a face outside React: the pill in the composer is
 * built by hand in the DOM, once per mention, and cannot wait on an effect.
 * It gets whatever the avatar component has already read into the cache -
 * which, by the time anyone types an `@`, is every agent the sidebar has
 * drawn - or the inline data URL an older workspace stored.
 */
export function cachedAvatarSrc(user) {
  const file = user?.avatarFile ?? null;
  const stamp = user?.avatarUpdatedAt ?? 0;
  const stored = String(user?.avatarUrl ?? "");
  const inline = stored.startsWith("data:") ? stored : null;
  if (!file) return inline;
  const key = `${file}@${stamp}`;
  return settled.has(key) ? (settled.get(key) ?? inline) : inline;
}

export function useAvatarSrc(user) {
  const workspace = useWorkspace();
  const file = user?.avatarFile ?? null;
  const stamp = user?.avatarUpdatedAt ?? 0;
  // Only a data URL is worth honouring. Workspaces written by an older build
  // hold a `blob:` string, which resolved in the document that minted it and
  // nowhere since - keeping it would show a Remove button for a photo that
  // cannot be displayed and was never really there.
  const stored = String(user?.avatarUrl ?? "");
  const inline = stored.startsWith("data:") ? stored : null;

  const key = file ? `${file}@${stamp}` : null;
  // Seeded from the cache rather than filled in by an effect, so a picture that
  // has already been read draws on the first frame instead of flashing the
  // initials underneath it every time a list is scrolled past.
  const [src, setSrc] = React.useState(() => (key && settled.has(key) ? settled.get(key) : inline));

  React.useEffect(() => {
    // A workspace written before the photo became a file still holds the data
    // URL. Honouring it means nobody loses their picture to an upgrade.
    if (!file) {
      setSrc(inline);
      return undefined;
    }
    if (settled.has(key)) {
      setSrc(settled.get(key));
      return undefined;
    }
    if (typeof workspace?.readBytes !== "function") {
      setSrc(inline);
      return undefined;
    }

    let alive = true;
    workspace
      .readBytes(file)
      .then((base64) => {
        const value = base64 ? `data:${mimeFor(file)};base64,${base64}` : null;
        remember(key, value);
        if (alive) setSrc(value);
      })
      .catch(() => {
        remember(key, null);
        if (alive) setSrc(null);
      });

    return () => {
      alive = false;
    };
  }, [file, stamp, key, inline, workspace]);

  return src;
}
