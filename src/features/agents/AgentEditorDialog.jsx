import * as React from "react";
import { Camera, Trash2, X, Icon } from "@/components/icons";
import { ANY, asRules, evaluate } from "@shared/permission";
import { groupedTools } from "@shared/tools";
import { thinkingControl } from "@shared/thinking";
import { useApp } from "@/lib/store";
import { useWorkspace } from "@/lib/workspace";
import {
  clearAgentPicture,
  saveAgentPicture,
  useAvatarSrc,
} from "@/lib/avatar";
import { formatBytes } from "@/data";
import { useToast } from "@/components/ui/toast";
import { Avatar } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogBody,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { SPAWN_DEFAULTS, spawnPolicy } from "@shared/crew";

/* Settings-band density: 28px controls, 12px labels, 11px descriptions. */

function Field({ label, description, htmlFor, children, className }) {
  return (
    <div className={cn("flex flex-col", className)}>
      <label htmlFor={htmlFor} className="text-xs text-foreground/90">
        {label}
      </label>
      <div className="mt-1">{children}</div>
      {description ? (
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          {description}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The models this workspace can actually reach.
 *
 * This list used to come from the demo catalogue, so a fresh install offered
 * Claude, GPT, Gemini and two local Llamas - none of which the user had
 * configured, none of which could answer, and all of which looked like real
 * choices. An agent pointed at one of them silently fell back to the workspace
 * default at send time, which is a picker that lies about what it does.
 *
 * The first option is always the workspace default. An agent that names no
 * model is not misconfigured: it is the normal case, and it means the agent
 * follows whatever the workspace is set to rather than pinning a choice made
 * once and forgotten.
 */
function modelOptions(chatModels, defaultLabel) {
  const groups = new Map();
  for (const model of chatModels ?? []) {
    const name = model.providerName || "Configured";
    if (!groups.has(name)) groups.set(name, []);
    groups.get(name).push({
      value: model.ref,
      label: model.label,
      description: model.id,
    });
  }
  return [
    {
      group: "Default",
      options: [
        {
          value: INHERIT,
          label: "Workspace default",
          description: defaultLabel || "Set under Settings, Providers",
        },
      ],
    },
    ...[...groups.entries()].map(([name, options]) => ({
      group: name,
      options,
    })),
  ];
}

/** An agent with no model of its own follows the workspace. */
const INHERIT = "";

/** The same ceiling the user's own photo has, for the same reason. */
const PICTURE_LIMIT_BYTES = 10 * 1024 * 1024;

/**
 * A picked file as a data URL, which is how the bytes reach both the preview
 * and the workspace. An object URL is cheaper and only resolves in the document
 * that minted it, so it could be shown and never saved.
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

/** Every tool the app knows about, flattened out of the one catalogue. */
const TOOL_ROWS = groupedTools().flatMap((group) => group.tools);

/**
 * Which tools an agent is currently denied outright.
 *
 * Only a blanket deny counts. A rule that denies one pattern is a nuance this
 * form has no way to show, and reading it as "off" would silently widen it to
 * everything the moment somebody pressed Save.
 */
function deniedFor(rules) {
  return TOOL_ROWS.filter((tool) => {
    const verdict = evaluate(rules ?? [], tool.key, ANY);
    return verdict.action === "deny" && (verdict.rule?.pattern ?? ANY) === ANY;
  }).map((tool) => tool.key);
}

/**
 * Fold the form's on/off answers back into the agent's rules.
 *
 * Everything the Permissions tab wrote is preserved except the blanket denies
 * this form owns, which are rewritten from scratch. That is what stops a create
 * form from quietly deleting a carefully written pattern rule.
 */
function withDenies(rules, denied) {
  const owned = new Set(TOOL_ROWS.map((tool) => tool.key));
  const kept = (rules ?? []).filter(
    (rule) =>
      !(
        owned.has(rule.tool) &&
        (rule.pattern ?? ANY) === ANY &&
        rule.action === "deny"
      ),
  );
  return [
    ...kept,
    ...denied.map((tool) => ({ tool, pattern: ANY, action: "deny" })),
  ];
}

/**
 * The delegation switches, in the order they stop mattering.
 *
 * `subagents` gates the other two: an agent that may not delegate at all cannot
 * meaningfully be allowed to nest, and showing those as live controls under an
 * off switch is a form that lies about what it will do.
 */
const SPAWN_ROWS = [
  {
    key: "subagents",
    label: "Use subagents",
    description:
      "May hand work to a temporary helper and keep working while it runs.",
  },
  {
    key: "agents",
    label: "Start other agents",
    description:
      "May run one of your configured agents, not only an anonymous helper.",
  },
  {
    key: "recursive",
    label: "Helpers may delegate",
    description:
      "Its own subagents may spawn subagents. Off by default: a tree grows fast.",
  },
];

const EMPTY = {
  name: "",
  handle: "",
  role: "",
  description: "",
  systemPrompt: "",
  model: INHERIT,
  thinkingBudget: 0,
  reasoningEffort: null,
  computerId: null,
  deniedTools: [],
  spawn: { ...SPAWN_DEFAULTS },
  tags: [],
};

export function AgentEditorDialog({ open, onOpenChange, agentId }) {
  const {
    agents,
    computers,
    createAgent,
    saveAgent,
    chatModels,
    defaultModelRef,
    permissions,
    setAgentRules,
  } = useApp();
  const { toast } = useToast();
  const workspace = useWorkspace();

  const existing = agentId ? agents.find((b) => b.id === agentId) : null;

  // undefined: unchanged. null: cleared. A string: a new picture, unsaved.
  const [picked, setPicked] = React.useState(undefined);
  const fileRef = React.useRef(null);
  const savedPicture = useAvatarSrc(existing);
  const pictureUrl = picked === undefined ? savedPicture : picked;

  const models = React.useMemo(() => {
    const fallback = chatModels?.find((m) => m.ref === defaultModelRef);
    return modelOptions(
      chatModels,
      fallback ? `Currently ${fallback.label}` : "",
    );
  }, [chatModels, defaultModelRef]);
  const [draft, setDraft] = React.useState(EMPTY);
  const [tagDraft, setTagDraft] = React.useState("");

  // reset the form every time the dialog is (re)opened for a subject
  React.useEffect(() => {
    if (!open) return;
    setTagDraft("");
    setPicked(undefined);
    setDraft(
      existing
        ? {
            name: existing.name ?? "",
            handle: existing.handle ?? "",
            role: existing.role ?? "",
            description: existing.description ?? "",
            systemPrompt: existing.systemPrompt ?? "",
            computerId: existing.computerId ?? null,
            thinkingBudget: Number(existing.thinkingBudget) || 0,
            reasoningEffort: existing.reasoningEffort ?? null,
            deniedTools: deniedFor(asRules(permissions?.agents?.[existing.id])),
            spawn: spawnPolicy(existing),
            // A stored model no configured provider offers is not a choice,
            // it is a leftover. Showing it selected would be a picker that
            // cannot send, so it reads as the workspace default, which is what
            // the turn was falling back to anyway.
            model: chatModels?.some((m) => m.ref === existing.model)
              ? existing.model
              : INHERIT,
            tags: existing.tags ?? [],
          }
        : EMPTY,
    );
  }, [open, agentId]); // eslint-disable-line react-hooks/exhaustive-deps

  const patch = (next) => setDraft((d) => ({ ...d, ...next }));

  const toggleCapability = (key) =>
    setDraft((d) => ({
      ...d,
      deniedTools: d.deniedTools.includes(key)
        ? d.deniedTools.filter((c) => c !== key)
        : [...d.deniedTools, key],
    }));

  const addTag = (raw) => {
    const tag = raw.trim().replace(/^#/, "").toLowerCase();
    if (!tag) return;
    setDraft((d) =>
      d.tags.includes(tag) ? d : { ...d, tags: [...d.tags, tag] },
    );
    setTagDraft("");
  };

  const removeTag = (tag) =>
    setDraft((d) => ({ ...d, tags: d.tags.filter((t) => t !== tag) }));

  const onTagKeyDown = (e) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      addTag(tagDraft);
    } else if (e.key === "Backspace" && !tagDraft && draft.tags.length) {
      e.preventDefault();
      removeTag(draft.tags[draft.tags.length - 1]);
    }
  };

  const valid = draft.name.trim().length > 0;

  /**
   * The thinking control for the model this agent will actually talk to.
   *
   * Read off the chosen model, or the workspace default when the agent
   * inherits it, the way opencode derives a model's variants: Claude gets a
   * token budget, an OpenAI-style reasoning model gets an effort level, and a
   * model with no such dial gets told so rather than a picker that changes
   * nothing. Off by default in every case, because thinking is not free.
   */
  const chosen = React.useMemo(() => {
    const ref =
      draft.model && draft.model !== INHERIT ? draft.model : defaultModelRef;
    return chatModels?.find((m) => m.ref === ref) ?? null;
  }, [draft.model, defaultModelRef, chatModels]);
  const thinking = React.useMemo(
    () => thinkingControl(chosen?.id, chosen?.protocol ?? "openai"),
    [chosen],
  );
  const thinkingValue =
    thinking.kind === "budget"
      ? String(draft.thinkingBudget ?? 0)
      : thinking.kind === "effort"
        ? (draft.reasoningEffort ?? "default")
        : "none";
  const onThinking = (v) => {
    if (thinking.kind === "budget") patch({ thinkingBudget: Number(v) });
    else if (thinking.kind === "effort")
      patch({ reasoningEffort: v === "default" ? null : v });
  };

  async function pickPicture(event) {
    const file = event.target.files?.[0];
    // Cleared first, so picking the same file twice still fires a change.
    event.target.value = "";
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      toast({
        variant: "danger",
        title: "That is not an image",
        description: "Pick a PNG, JPEG, GIF or WebP.",
      });
      return;
    }
    if (file.size > PICTURE_LIMIT_BYTES) {
      toast({
        variant: "danger",
        title: "That picture is too large",
        description: `Pictures are capped at 2 MB, and that one is ${formatBytes(file.size)}.`,
      });
      return;
    }
    try {
      setPicked(await readAsDataUrl(file));
    } catch {
      toast({ variant: "danger", title: "Could not read that picture" });
    }
  }

  /**
   * The picture, written to the folder before the record points at it.
   *
   * A new agent has no id until it exists, and the file is named after the id,
   * so this runs after the record is made and patches it. Which also means a
   * folder that cannot be written costs the picture and never the agent.
   */
  async function commitPicture(id, previousFile) {
    if (picked === undefined) return {};
    return picked
      ? saveAgentPicture(workspace, id, picked, previousFile)
      : clearAgentPicture(workspace, { avatarFile: previousFile });
  }

  const submit = () => {
    if (!valid) return;
    const handle = draft.handle.trim()
      ? draft.handle.trim().startsWith("@")
        ? draft.handle.trim()
        : "@" + draft.handle.trim()
      : "@" + draft.name.trim().toLowerCase().replace(/\s+/g, "");

    const { deniedTools, ...rest } = draft;
    const payload = { ...rest, name: draft.name.trim(), handle };

    if (existing) {
      saveAgent(existing.id, payload);
      setAgentRules(existing.id, (current) => withDenies(current, deniedTools));
      commitPicture(existing.id, existing.avatarFile)
        .then((patch) => {
          if (Object.keys(patch).length) saveAgent(existing.id, patch);
        })
        .catch((error) =>
          toast({
            variant: "danger",
            title: "The picture could not be saved",
            description:
              error?.message ?? "The workspace folder could not be written to.",
          }),
        );
      toast({ title: `${payload.name} saved`, variant: "success" });
    } else {
      const id = createAgent(payload);
      // The rules need the id, which only exists once the agent does.
      if (deniedTools.length)
        setAgentRules(id, (current) => withDenies(current, deniedTools));
      // And so does the picture: the file is named after the agent.
      commitPicture(id, null)
        .then((patch) => {
          if (Object.keys(patch).length) saveAgent(id, patch);
        })
        .catch(() => {});
      toast({
        title: `${payload.name} created`,
        description: "Give it a computer and a routine to put it to work.",
        variant: "success",
      });
    }
    onOpenChange?.(false);
  };

  // A workspace with no machines in it is the common case on a fresh install,
  // and "No computer / Chat only" said nothing about how to change that - so
  // the picker looked broken to anyone who had just set a provider up. A
  // provider being ready means a machine CAN be made, not that one exists.
  const hasComputers = Boolean(computers?.length);

  const computerChoices = [
    {
      value: "__none",
      label: "No computer",
      description: hasComputers
        ? "Chat only - no terminal, files or browser."
        : "No machines exist yet. Make one on the Computers screen first.",
    },
    // The machines the workspace actually has. There is no seed list to fall
    // back on any more, and there should not be: offering a computer that does
    // not exist is how an agent ends up assigned to nothing.
    ...(computers ?? []).map((c) => ({
      value: c.id,
      label: c.name,
      icon: <Icon name="Monitor" />,
      description: [c.provider, c.status].filter(Boolean).join(" · "),
    })),
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      size="lg"
      className="max-h-[calc(100dvh-4rem)]"
    >
      <DialogTitle>
        {existing ? `Edit ${existing.name}` : "New agent"}
      </DialogTitle>

      <DialogBody className="min-h-0">
        <ScrollArea className="max-h-[min(32rem,calc(100dvh-16rem))] -mx-1 px-1">
          <div className="flex flex-col gap-4 pb-1">
            {/* A face, for the same reason the user has one: eight teammates
                that differ by two letters in a circle are eight teammates
                nobody can tell apart in a sidebar. */}
            <div className="flex flex-wrap items-center gap-3">
              <Avatar
                src={pictureUrl ?? undefined}
                name={draft.name || "New agent"}
                size="xl"
                icon={draft.icon ? <Icon name={draft.icon} /> : undefined}
              />
              <div className="flex flex-wrap items-center gap-1.5">
                <Button
                  variant="subtle"
                  size="xs"
                  onClick={() => fileRef.current?.click()}
                >
                  <Camera className="size-3.5" aria-hidden="true" />
                  {pictureUrl ? "Replace picture" : "Upload picture"}
                </Button>
                {pictureUrl ? (
                  <Button
                    variant="ghost"
                    size="xs"
                    className="animate-fade-in"
                    onClick={() => setPicked(null)}
                  >
                    <Trash2 className="size-3.5" aria-hidden="true" />
                    Remove
                  </Button>
                ) : null}
                <input
                  ref={fileRef}
                  type="file"
                  accept="image/*"
                  className="hidden"
                  onChange={pickPicture}
                  aria-hidden="true"
                  tabIndex={-1}
                />
              </div>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="Name" htmlFor="agent-name">
                <Input
                  id="agent-name"
                  size="xs"
                  value={draft.name}
                  placeholder="Atlas"
                  onChange={(e) => patch({ name: e.target.value })}
                />
              </Field>
              <Field
                label="Handle"
                description="Used to address it in a thread."
                htmlFor="agent-handle"
              >
                <Input
                  id="agent-handle"
                  size="xs"
                  value={draft.handle}
                  placeholder="@atlas"
                  onChange={(e) => patch({ handle: e.target.value })}
                />
              </Field>
            </div>

            <Field label="Role" htmlFor="agent-role">
              <Input
                id="agent-role"
                size="xs"
                value={draft.role}
                placeholder="Research analyst"
                onChange={(e) => patch({ role: e.target.value })}
              />
            </Field>

            <Field
              label="Description"
              description="One paragraph a teammate can read to know what this agent is for."
              htmlFor="agent-description"
            >
              <Textarea
                id="agent-description"
                rows={3}
                className="text-xs"
                value={draft.description}
                placeholder="What does this teammate do?"
                onChange={(e) => patch({ description: e.target.value })}
              />
            </Field>

            <Field
              label="System prompt"
              description="The standing instructions. Operating rules and hard stops belong here."
              htmlFor="agent-prompt"
            >
              <Textarea
                id="agent-prompt"
                autoResize
                maxRows={14}
                rows={6}
                className="font-mono text-xs leading-relaxed"
                value={draft.systemPrompt}
                placeholder="You are…"
                onChange={(e) => patch({ systemPrompt: e.target.value })}
              />
            </Field>

            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="Model">
                <Select
                  size="xs"
                  ariaLabel="Model"
                  value={draft.model ?? INHERIT}
                  options={models}
                  onChange={(v) => patch({ model: v })}
                />
              </Field>
              <Field label="Thinking">
                <Select
                  size="xs"
                  ariaLabel="Thinking"
                  value={thinkingValue}
                  options={thinking.options}
                  disabled={thinking.kind === "none"}
                  onChange={onThinking}
                />
              </Field>
              <Field label="Computer">
                <Select
                  size="xs"
                  ariaLabel="Computer"
                  value={draft.computerId ?? "__none"}
                  options={computerChoices}
                  onChange={(v) =>
                    patch({ computerId: v === "__none" ? null : v })
                  }
                />
                {hasComputers ? null : (
                  <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                    Nothing to assign yet. Computers, then New computer - a
                    Docker container here, or a sandbox in the cloud.
                  </p>
                )}
              </Field>
            </div>

            <div>
              {/* These used to be "Capabilities": seven invented names, four of
                  which (Browser, Desktop, Voice, Web search) matched no tool
                  that exists, and none of which anything read. An agent's tools
                  are decided by its permission rules, so these are those rules,
                  at the only resolution a create form should ask for. The
                  Permissions tab on the agent is where patterns and Ask live. */}
              <p className="text-xs text-foreground/90">Tools</p>
              <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                What this agent may reach for. Switching one off hides it from
                the model entirely, rather than letting it try and be refused.
              </p>
              <div className="mt-1.5 grid gap-x-4 gap-y-1 sm:grid-cols-2">
                {TOOL_ROWS.map((tool) => {
                  const id = `tool-${tool.key}`;
                  const checked = !draft.deniedTools.includes(tool.key);
                  return (
                    <label
                      key={tool.key}
                      htmlFor={id}
                      title={tool.description}
                      className={cn(
                        "flex h-7 cursor-pointer items-center gap-2 rounded-lg px-2",
                        "transition-colors duration-150 ease-out hover:fill-nav",
                      )}
                    >
                      <Checkbox
                        id={id}
                        size="sm"
                        checked={checked}
                        onCheckedChange={() => toggleCapability(tool.key)}
                      />
                      <Icon
                        name={tool.icon ?? "Wrench"}
                        aria-hidden="true"
                        className={cn(
                          "size-3.5",
                          checked ? "text-foreground" : "text-muted-foreground",
                        )}
                      />
                      <span className="text-xs text-foreground/90">
                        {tool.label}
                      </span>
                    </label>
                  );
                })}
              </div>
            </div>

            <div>
              {/* Separate from Tools, because it is not a tool but a policy about
                  what this agent may make. An agent that can spawn is an agent
                  that can spend without a further decision from anyone, and the
                  switch for that should not be one checkbox in a grid of eight. */}
              <p className="text-xs text-foreground/90">Delegation</p>
              <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                Whether this agent may build a team. There is no cap on how many
                it starts unless you set one - what a run costs is shown while
                it runs, in the panel beside the chat.
              </p>
              <div className="mt-1.5 grid gap-x-4 gap-y-1 sm:grid-cols-2">
                {SPAWN_ROWS.map((row) => {
                  const id = `spawn-${row.key}`;
                  const checked = Boolean(draft.spawn?.[row.key]);
                  return (
                    <label
                      key={row.key}
                      htmlFor={id}
                      title={row.description}
                      className={cn(
                        "flex h-7 cursor-pointer items-center gap-2 rounded-lg px-2",
                        "transition-colors duration-150 ease-out hover:fill-nav",
                      )}
                    >
                      <Checkbox
                        id={id}
                        size="sm"
                        checked={checked}
                        disabled={
                          row.key !== "subagents" && !draft.spawn?.subagents
                        }
                        onCheckedChange={() =>
                          patch({
                            spawn: { ...draft.spawn, [row.key]: !checked },
                          })
                        }
                      />
                      <span className="text-xs text-foreground/90">
                        {row.label}
                      </span>
                    </label>
                  );
                })}
                <label
                  htmlFor="spawn-max"
                  className="flex h-7 items-center gap-2 rounded-lg px-2"
                  title="How many of its own runs may be live at once. Blank means no limit."
                >
                  <span className="text-xs text-foreground/90">At once</span>
                  <Input
                    id="spawn-max"
                    type="number"
                    min="0"
                    inputMode="numeric"
                    className="h-6 w-16 text-xs"
                    placeholder="No limit"
                    disabled={!draft.spawn?.subagents}
                    value={
                      draft.spawn?.maxConcurrent
                        ? String(draft.spawn.maxConcurrent)
                        : ""
                    }
                    onChange={(e) =>
                      patch({
                        spawn: {
                          ...draft.spawn,
                          maxConcurrent: Number(e.target.value) || 0,
                        },
                      })
                    }
                  />
                </label>
              </div>
            </div>

            <Field
              label="Tags"
              description="Enter to add, Backspace to remove the last one."
              htmlFor="agent-tags"
            >
              <div className="flex flex-wrap items-center gap-1.5">
                {draft.tags.map((tag) => (
                  <span
                    key={tag}
                    className="inline-flex h-6 animate-pop-in items-center gap-1 rounded-full fill-secondary pr-1 pl-2.5 text-[11px] text-muted-foreground"
                  >
                    {tag}
                    <button
                      type="button"
                      aria-label={`Remove ${tag}`}
                      onClick={() => removeTag(tag)}
                      className={cn(
                        "grid size-4 place-items-center rounded-full outline-none",
                        "transition-colors duration-150 ease-out hover:fill-close hover:text-foreground",
                        "focus-visible:fill-close",
                      )}
                    >
                      <X className="size-3" aria-hidden="true" />
                    </button>
                  </span>
                ))}
                <Input
                  id="agent-tags"
                  size="xs"
                  className="w-40"
                  value={tagDraft}
                  placeholder="Add a tag"
                  onChange={(e) => setTagDraft(e.target.value)}
                  onKeyDown={onTagKeyDown}
                  onBlur={() => addTag(tagDraft)}
                />
              </div>
            </Field>
          </div>
        </ScrollArea>
      </DialogBody>

      <DialogFooter>
        <Button
          variant="secondary"
          size="pill"
          onClick={() => onOpenChange?.(false)}
        >
          Cancel
        </Button>
        <Button
          variant="primary"
          size="pill"
          disabled={!valid}
          onClick={submit}
        >
          {existing ? "Save changes" : "Create agent"}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
