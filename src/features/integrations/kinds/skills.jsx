import * as React from "react";
import { FileText, FolderOpen, RotateCcw, Sparkles } from "@/components/icons";
import { useWorkspace } from "@/lib/workspace";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Field, TagsInput, ToggleRow } from "../fields";

/**
 * Skills.
 *
 * A skill is a FOLDER, not a file. `skills/<slug>/SKILL.md` is the part Inertia
 * writes and reads back through frontmatter, but the folder is the unit: a
 * script the procedure tells the agent to run, a template it fills in, a
 * reference table it quotes from all belong beside it, and they travel with the
 * skill when the workspace is copied. The screen says so out loud, because a
 * dialog with one big text box in it teaches the opposite.
 */

const blank = () => ({
  name: "",
  description: "",
  enabled: true,
  tags: [],
  version: "",
  instructions: "",
  extra: {},
});

/** Mirrors `slugify` in main/workspace/collections.cjs, which decides for real. */
function previewSlug(name) {
  return (
    String(name ?? "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64) || "skill"
  );
}

/** Bytes, in the shortest form that is still honest about the size. */
function sizeOf(bytes) {
  if (bytes == null) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Where the skill lives, and what is in it.
 *
 * Inertia only ever writes SKILL.md, but the folder is the unit, so listing
 * only the file we wrote would misrepresent what the user has: a skill whose
 * instructions say "run scripts/check.py" is broken if that script is missing,
 * and a screen that cannot show it cannot show that either.
 *
 * So the folder is read from disk. Everything beside SKILL.md is marked as the
 * user's, because that is the contract - Inertia never edits or deletes those
 * files, and saying so is what makes the folder safe to put things in.
 */
function SkillFolder({ id, name }) {
  const { native, configured, reveal, listDir } = useWorkspace();
  const folder = `skills/${id ?? previewSlug(name)}`;
  const [entries, setEntries] = React.useState(null);

  const load = React.useCallback(() => {
    if (!id || !configured || typeof listDir !== "function") return;
    listDir(folder)
      .then((rows) => setEntries(Array.isArray(rows) ? rows : []))
      .catch(() => setEntries([]));
  }, [id, folder, configured, listDir]);

  React.useEffect(load, [load]);

  // Only what the user put there. SKILL.md gets its own row above, because it
  // is the one file this form owns and mixing it in would blur that.
  const theirs = (entries ?? []).filter((entry) => entry.name !== "SKILL.md");

  return (
    <div className="flex flex-col gap-2 rounded-xl fill-whisper px-3 py-2.5">
      <div className="flex items-center gap-2">
        <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <p className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground" title={folder}>
          {folder}/
        </p>
        {native && configured && id ? (
          <>
            <Button variant="ghost" size="xs" onClick={load} aria-label="Refresh the file list">
              <RotateCcw />
            </Button>
            <Button variant="subtle" size="xs" onClick={() => reveal(folder)}>
              <FolderOpen />
              Open folder
            </Button>
          </>
        ) : null}
      </div>

      <div className="flex flex-col gap-1 pl-5">
        <div className="flex items-center gap-2">
          <FileText className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <p className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">SKILL.md</p>
          <span className="shrink-0 text-[11px] text-muted-foreground">written from this form</span>
        </div>

        {theirs.map((entry) => (
          <div key={entry.name} className="flex animate-fade-in items-center gap-2">
            {entry.isDir ? (
              <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            ) : (
              <FileText className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            )}
            <p className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">
              {entry.name}
              {entry.isDir ? "/" : ""}
            </p>
            <span className="shrink-0 tabular-nums text-[11px] text-muted-foreground">
              {entry.isDir ? "folder" : sizeOf(entry.size)}
            </span>
          </div>
        ))}
      </div>

      <p className="pl-5 text-[11px] leading-relaxed text-muted-foreground">
        {!id
          ? "The folder is created when the skill is saved. Its name comes from the skill's name, and it is where any scripts or templates the procedure refers to should go."
          : theirs.length
            ? "Those files are yours. Inertia never edits or deletes them, and they are copied with the skill when the workspace moves. Refer to them by relative path in the instructions below."
            : "Nothing else here yet. Put a script, a template or a reference table in this folder and refer to it by relative path in the instructions below - it travels with the skill."}
      </p>
    </div>
  );
}

/**
 * What is wrong with the skill on disk, as the main process read it.
 *
 * The failure this exists for is silent: a skill with no description is loaded,
 * enabled and never once offered to an agent, and nothing anywhere says so. The
 * list is computed where the file is parsed, so this shows what the agent
 * actually got rather than re-deciding it from the form.
 */
function SkillProblems({ problems }) {
  if (!problems?.length) return null;
  return (
    <div className="flex flex-col gap-1 rounded-xl bg-warning-wash px-3 py-2.5">
      {problems.map((problem) => (
        <p key={problem} className="text-[11px] leading-relaxed text-warning-ink">
          {problem}
        </p>
      ))}
    </div>
  );
}

function SkillForm({ value, onChange, editing }) {
  const set = (patch) => onChange(patch);

  return (
    <div className="flex flex-col gap-4">
      <SkillProblems problems={editing ? value.problems : null} />
      <SkillFolder id={editing ? value.id : null} name={value.name} />

      <div className="grid gap-4 sm:grid-cols-[2fr_1fr]">
        <Field label="Name" hint={editing ? null : "The folder name is taken from this."}>
          <Input
            size="sm"
            value={value.name}
            placeholder="Release notes"
            onChange={(e) => set({ name: e.target.value })}
          />
        </Field>
        <Field label="Version" hint="Optional.">
          <Input
            size="sm"
            className="font-mono"
            value={value.version}
            placeholder="1.0.0"
            onChange={(e) => set({ version: e.target.value })}
          />
        </Field>
      </div>

      <Field
        label="Description"
        hint="One line. This is what an agent reads when deciding whether the skill applies."
      >
        <Textarea
          rows={2}
          autoResize
          maxRows={3}
          value={value.description}
          placeholder="Turns a range of merged pull requests into release notes."
          onChange={(e) => set({ description: e.target.value })}
        />
      </Field>

      <Field label="Tags" hint="Comma separated.">
        <TagsInput value={value.tags} onChange={(tags) => set({ tags })} />
      </Field>

      <Field label="Instructions" hint="Markdown. This is the body of SKILL.md.">
        <Textarea
          rows={14}
          autoResize
          maxRows={26}
          className="font-mono text-[12px] leading-relaxed"
          value={value.instructions}
          placeholder={"## When to use\n\n…\n\n## Steps\n\n1. …"}
          onChange={(e) => set({ instructions: e.target.value })}
        />
      </Field>

      <ToggleRow
        label="Enabled"
        description="A disabled skill stays in the folder and is not offered to any agent."
        checked={value.enabled !== false}
        onCheckedChange={(enabled) => set({ enabled })}
      />
    </div>
  );
}

export const SKILLS_KIND = {
  id: "skills",
  collection: "skills",
  label: "Skills",
  icon: Sparkles,
  noun: "skill",
  nounPlural: "skills",
  listTitle: "Skills",
  addLabel: "Add skill",
  dialogSize: "xl",
  dialogHint:
    "Saved as a folder of its own, with SKILL.md inside it. Whatever else the procedure needs goes in that folder beside it.",
  searchPlaceholder: "Search skills",
  empty: {
    title: "No skills yet",
    description:
      "A skill is a piece of written procedure an agent can follow: how you cut a release, how you triage a bug. Each one lives as a markdown file in the workspace folder.",
  },
  blank,
  toDraft: (record) => ({ ...blank(), ...record, tags: record.tags ?? [] }),
  toRecord: (draft) => ({
    name: draft.name.trim(),
    description: draft.description?.trim() ?? "",
    enabled: draft.enabled !== false,
    tags: draft.tags ?? [],
    version: draft.version?.trim() ?? "",
    instructions: draft.instructions ?? "",
    // Frontmatter keys this form does not show. A save writes the whole file,
    // so anything the user hand-wrote and we did not carry back would be gone.
    extra: draft.extra ?? {},
  }),
  validate: (draft) => {
    if (!draft.name.trim()) return "Give the skill a name.";
    // Not optional, whatever the frontmatter allows. The description is the
    // only thing an agent sees before it chooses, so a skill saved without one
    // is a skill that will never be used.
    if (!draft.description?.trim()) {
      return "Write a description. It is the only thing an agent reads when deciding whether this skill applies.";
    }
    if (!draft.instructions.trim()) return "A skill with no instructions has nothing to teach.";
    return null;
  },
  search: (record) =>
    [record.name, record.description, (record.tags ?? []).join(" "), record.instructions].join(" "),
  row: (record) => ({
    glyph: Sparkles,
    title: record.name || record.id,
    subtitle: record.problems?.length ? record.problems[0] : record.description || `skills/${record.id}/`,
    badges: [
      // First, and in the loud colour. A skill that no agent will ever reach
      // for looks exactly like a working one in this list, which is the whole
      // reason a broken skill can sit there for weeks.
      ...(record.problems?.length ? [{ label: "Needs attention", variant: "warning" }] : []),
      ...(record.tags ?? []).slice(0, 2).map((tag) => ({ label: tag })),
    ],
  }),
  removeHint: (record) =>
    `The whole skills/${record.id} folder is deleted, SKILL.md and all. Nothing else in the workspace refers to it afterwards.`,
  Form: SkillForm,
};
