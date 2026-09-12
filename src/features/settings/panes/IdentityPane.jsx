import * as React from "react";
import {
  Camera,
  Check,
  FolderOpen,
  Trash2,
  UserRound,
} from "@/components/icons";
import { formatBytes } from "@/data";
import { useApp } from "@/lib/store";
import { useWorkspace } from "@/lib/workspace";
import { clearAvatar, saveAvatar, useAvatarSrc } from "@/lib/avatar";
import { Avatar } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useToast } from "@/components/ui/toast";
import {
  SettingsCard,
  SettingsField,
  SettingsRow,
  SettingsSection,
} from "../SettingsRow";

const BIO_LIMIT = 600;

/**
 * Big enough for any sane avatar, small enough that the picked image can be
 * held in memory as base64 and handed across the IPC bridge in one message.
 */
const AVATAR_LIMIT_BYTES = 10 * 1024 * 1024;

/**
 * Read a picked file as a data URL, which is how the bytes reach both the
 * preview and `saveAvatar`. An object URL would be cheaper, but it only
 * resolves inside the document that minted it, so it could not be written to
 * the workspace and could not survive a reload.
 */
function readAsDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () =>
      reject(reader.error ?? new Error("Could not read that file"));
    reader.readAsDataURL(file);
  });
}

/** Two initials derived from whatever is currently typed in the name field. */
function initialsFrom(name) {
  const words = String(name ?? "")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (!words.length) return "";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[words.length - 1][0]).toUpperCase();
}

/**
 * What the agent actually reads.
 *
 * This mirrors `person()` in `src/main/agent/prompt.cjs`, which is the code
 * that really assembles it. Two copies of a sentence is a small price for the
 * user being able to see the prompt they are writing: a bio field with no
 * preview is a text box you fill in and hope about.
 */
/** The Region settings say "system" until somebody picks. See `settingOf`. */
function chosen(value) {
  const text = String(value ?? "").trim();
  return text && text !== "system" ? text : null;
}

function promptPreview({
  name,
  shortName,
  handle,
  bio,
  email,
  timezone,
  locale,
}) {
  const who = String(name ?? "").trim();
  const short = String(shortName ?? "").trim();
  const at = String(handle ?? "").trim();
  const about = String(bio ?? "").trim();
  const address = String(email ?? "").trim();
  const zone = chosen(timezone);
  const format = chosen(locale);
  if (!who && !about && !address && !zone && !format) return null;

  const lines = [];
  if (who) {
    const full = at ? `${who} (${at})` : who;
    lines.push(
      short && short.toLowerCase() !== who.toLowerCase()
        ? `You work for ${full}. Call them ${short}.`
        : `You work for ${full}.`,
    );
  }
  if (!who && (address || zone || format)) {
    lines.push("You work for someone who has not filled in their name.");
  }
  if (address)
    lines.push(
      `Their email address is ${address}. Do not use it anywhere they did not ask you to.`,
    );
  if (zone)
    lines.push(
      `They are in the ${zone} time zone. Write times in it unless asked otherwise.`,
    );
  if (format)
    lines.push(`They read dates and numbers in the ${format} format.`);
  if (about) {
    lines.push("");
    lines.push("They describe themselves and how they want to be worked with:");
    lines.push("");
    lines.push(about);
  }
  return lines.join("\n");
}

export function IdentityPane() {
  const { user, updateProfile } = useApp();
  const workspace = useWorkspace();
  const { toast } = useToast();
  const fileRef = React.useRef(null);

  // What is on disk. The picker sets `picked` instead, so nothing is written
  // until the profile is saved and Revert really does revert.
  const savedAvatar = useAvatarSrc(user);
  const [picked, setPicked] = React.useState(undefined);
  const avatarUrl = picked === undefined ? savedAvatar : picked;

  const [name, setName] = React.useState(user.name);
  const [shortName, setShortName] = React.useState(user.shortName ?? "");
  const [handle, setHandle] = React.useState(user.handle);
  const [email, setEmail] = React.useState(user.email);
  const [initials, setInitials] = React.useState(user.avatarInitials);
  const [bio, setBio] = React.useState(user.bio ?? "");
  const [saving, setSaving] = React.useState(false);

  // The region settings are set on the General tab and live in the same file,
  // so the preview shows them: they are part of what an agent is told about the
  // person, and a preview that leaves them out is a preview of a different
  // prompt to the one that gets sent.
  const preview = promptPreview({
    name,
    shortName,
    handle,
    bio,
    email,
    timezone: user.preferences?.timezone,
    locale: user.preferences?.locale,
  });

  const dirty =
    name !== user.name ||
    shortName !== (user.shortName ?? "") ||
    handle !== user.handle ||
    email !== user.email ||
    initials !== user.avatarInitials ||
    picked !== undefined ||
    bio !== (user.bio ?? "");

  async function pickPhoto(event) {
    const file = event.target.files?.[0];
    // clear the input first, so picking the same file again still fires change
    event.target.value = "";
    if (!file) return;

    // the picker is filtered to image/*, but a drag or a stubborn OS dialog can
    // still hand us something else
    if (!file.type.startsWith("image/")) {
      toast({
        variant: "danger",
        title: "That is not an image",
        description: "Pick a PNG, JPEG, GIF or WebP.",
      });
      return;
    }

    if (file.size > AVATAR_LIMIT_BYTES) {
      toast({
        variant: "danger",
        title: "That photo is too large",
        description: `Avatars are capped at 2 MB, and that one is ${formatBytes(file.size)}.`,
      });
      return;
    }

    try {
      setPicked(await readAsDataUrl(file));
    } catch {
      toast({
        variant: "danger",
        title: "Could not read that photo",
        description:
          "The file may have moved or be unreadable. Try picking it again.",
      });
    }
  }

  function clearPhoto() {
    setPicked(null);
  }

  async function save() {
    const nextName = name.trim() || user.name;
    const nextHandle = handle.trim().replace(/^@*/, "");

    setSaving(true);
    try {
      // The photo is written to the folder first. If that fails the profile is
      // not saved either, because a record pointing at a file that is not there
      // is worse than an unsaved change the user can try again.
      let photo = {};
      if (picked !== undefined) {
        photo = picked
          ? await saveAvatar(workspace, picked)
          : await clearAvatar(workspace, user);
      }

      updateProfile({
        name: nextName,
        shortName: shortName.trim(),
        handle: nextHandle ? `@${nextHandle}` : user.handle,
        email: email.trim() || user.email,
        avatarInitials: (initials.trim() || initialsFrom(nextName))
          .slice(0, 2)
          .toUpperCase(),
        bio,
        ...photo,
      });
      setPicked(undefined);
      toast({
        title: "Profile saved",
        description: "Every agent reads this before its next reply.",
        variant: "success",
      });
    } catch (error) {
      toast({
        variant: "danger",
        title: "Could not save your profile",
        description:
          error?.message ?? "The workspace folder could not be written to.",
      });
    } finally {
      setSaving(false);
    }
  }

  function revert() {
    setName(user.name);
    setShortName(user.shortName ?? "");
    setHandle(user.handle);
    setEmail(user.email);
    setInitials(user.avatarInitials);
    setBio(user.bio ?? "");
    setPicked(undefined);
  }

  return (
    <div className="w-full">
      <SettingsSection
        title="Photo"
        description="Shown beside your messages and anywhere Inertia needs to tell you apart from an agent. Without one, your initials are used."
      >
        <div className="flex flex-wrap items-center gap-4 px-4 py-3">
          <Avatar
            src={avatarUrl ?? undefined}
            name={initials.trim() || name}
            size="xl"
            icon={initials.trim() || name ? undefined : <UserRound />}
          />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="flex flex-wrap items-center gap-1.5">
              <Button
                variant="subtle"
                size="xs"
                onClick={() => fileRef.current?.click()}
              >
                <Camera className="size-3.5" aria-hidden="true" />
                {avatarUrl ? "Replace photo" : "Upload photo"}
              </Button>
              {avatarUrl ? (
                <Button variant="ghost" size="xs" className="animate-fade-in" onClick={clearPhoto}>
                  <Trash2 className="size-3.5" aria-hidden="true" />
                  Remove
                </Button>
              ) : null}
              <input
                ref={fileRef}
                type="file"
                accept="image/*"
                className="hidden"
                onChange={pickPhoto}
                aria-hidden="true"
                tabIndex={-1}
              />
            </div>
            {/* The real recorded path, not a guess at it: the extension follows
                the image you picked, so a JPEG is not saved as avatar.png and
                telling you it was would send you looking for a file that is not
                there. */}
            <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
              A square PNG or JPEG works best. It is written to your workspace
              as
              <code className="mx-1 font-mono">
                {user.avatarFile ?? "settings/avatar.png"}
              </code>
              , so you can also just replace that file. If it is ever missing,
              your initials are drawn instead.
            </p>
          </div>
        </div>

        <SettingsRow
          label="Initials"
          htmlFor="set-initials"
          description="The two letters drawn when there is no photo."
          control={
            <div className="flex items-center gap-1.5">
              <Input
                id="set-initials"
                size="xs"
                className="w-16 "
                maxLength={2}
                value={initials}
                onChange={(e) => setInitials(e.target.value.toUpperCase())}
              />
              <Button
                variant="ghost"
                size="xs"
                disabled={initials === initialsFrom(name)}
                onClick={() => setInitials(initialsFrom(name))}
              >
                Use my name
              </Button>
            </div>
          }
        />
      </SettingsSection>

      <SettingsSection title="Profile">
        <SettingsRow
          label="Display name"
          htmlFor="set-display-name"
          description="How Inertia addresses you, and how you are signed in threads."
          control={
            <Input
              id="set-display-name"
              size="xs"
              className="w-56"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          }
        />
        <SettingsRow
          label="Short name"
          htmlFor="set-short-name"
          description="What agents call you in a sentence. Leave it empty and they use your display name."
          control={
            <Input
              id="set-short-name"
              size="xs"
              className="w-56"
              value={shortName}
              placeholder={name.trim().split(/\s+/)[0] ?? ""}
              onChange={(e) => setShortName(e.target.value)}
            />
          }
        />
        <SettingsRow
          label="Handle"
          htmlFor="set-handle"
          description="Used to mention yourself in a routine prompt or an agent instruction."
          control={
            <Input
              id="set-handle"
              size="xs"
              className="w-56"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
            />
          }
        />
        <SettingsRow
          label="Email"
          htmlFor="set-email"
          description="Where run summaries, routine failures and release notes are sent."
          control={
            <Input
              id="set-email"
              size="xs"
              type="email"
              autoComplete="off"
              className="w-56"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        flat
        title="About you"
        description="Every agent reads this before it answers, so it knows who it works for. Say what you do, how you like to be written to, and anything it should never assume. Plain sentences beat bullet points."
      >
        <SettingsField
          htmlFor="set-bio"
          description={`${bio.length} of ${BIO_LIMIT} characters. Leave it empty and agents start with no context about you.`}
        >
          <Textarea
            id="set-bio"
            autoResize
            rows={5}
            maxRows={16}
            maxLength={BIO_LIMIT}
            placeholder="I build desktop software on my own. Write to me plainly, skip the preamble, and ask before touching anything in production."
            value={bio}
            onChange={(e) => setBio(e.target.value)}
          />
        </SettingsField>
      </SettingsSection>

      <SettingsSection
        flat
        title="What agents read"
        description="Assembled from the fields above, plus the region settings on the General tab, and put in front of every agent on every turn. Your photo and initials are not in here: they change what you see and tell a model nothing it can act on."
      >
        {preview ? (
          <pre className="max-h-64 overflow-auto rounded-xl fill-whisper px-3 py-2.5 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-foreground">
            {preview}
          </pre>
        ) : (
          <p className="rounded-xl fill-whisper px-3 py-2.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
            Nothing yet. With no name and no description, agents are told
            nothing about who they work for and answer as if talking to a
            stranger.
          </p>
        )}
      </SettingsSection>

      <SettingsSection
        flat
        title="Where this lives"
        description="Your profile is a file in the workspace folder, not a database. Edit it in your own editor if you would rather, and drop in a new avatar.png any time."
      >
        <SettingsCard className="flex flex-wrap items-center gap-2 p-2">
          <p className="min-w-0 flex-1 font-mono text-[11px] break-all text-foreground">
            settings/identity.json
          </p>
          {workspace.native && workspace.configured ? (
            <Button
              variant="subtle"
              size="xs"
              onClick={() => workspace.reveal("settings")}
            >
              <FolderOpen className="size-3.5" aria-hidden="true" />
              Open folder
            </Button>
          ) : null}
        </SettingsCard>
      </SettingsSection>

      <SettingsSection title="Changes">
        <SettingsRow
          label={dirty ? "You have unsaved changes" : "Everything is saved"}
          description={
            dirty
              ? "Nothing above is applied until you save."
              : "Your profile matches what agents currently read."
          }
          control={
            <div className="flex items-center gap-1.5">
              <Button
                variant="ghost"
                size="xs"
                disabled={!dirty || saving}
                onClick={revert}
              >
                Revert
              </Button>
              <Button
                variant="primary"
                size="xs"
                disabled={!dirty || saving}
                onClick={save}
              >
                <Check className="size-3.5" aria-hidden="true" />
                Save profile
              </Button>
            </div>
          }
        />
      </SettingsSection>
    </div>
  );
}
