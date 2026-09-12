import * as React from "react";
import { X, Icon } from "@/components/icons";
import { MEMORY_KIND_META } from "@/data";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";

const KIND_OPTIONS = Object.entries(MEMORY_KIND_META).map(([value, meta]) => ({
  value,
  label: meta.label,
  icon: <Icon name={meta.icon} />,
}));

/**
 * Write a memory by hand. `agentId` pins it to one teammate (the agent detail case);
 * without it the author picks who remembers this.
 */
export function NewMemoryDialog({ open, onOpenChange, agentId, onCreated }) {
  const { agents, createMemory } = useApp();

  const [kind, setKind] = React.useState("fact");
  const [agent, setAgent] = React.useState(agentId ?? agents[0]?.id ?? null);
  const [title, setTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const [tags, setTags] = React.useState([]);
  const [tagDraft, setTagDraft] = React.useState("");
  const [pinned, setPinned] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    setKind("fact");
    setAgent(agentId ?? agents[0]?.id ?? null);
    setTitle("");
    setBody("");
    setTags([]);
    setTagDraft("");
    setPinned(false);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  const addTag = (raw) => {
    const tag = String(raw).trim().replace(/^#/, "").toLowerCase();
    if (!tag) return;
    setTags((prev) => (prev.includes(tag) ? prev : [...prev, tag]));
    setTagDraft("");
  };
  const removeTag = (tag) => setTags((prev) => prev.filter((t) => t !== tag));

  const trimmed = title.trim();
  const agentName = agents.find((b) => b.id === agent)?.name;

  function submit() {
    const id = createMemory({
      agentId: agent,
      kind,
      title: trimmed,
      body: body.trim(),
      tags,
      pinned,
    });
    onOpenChange(false);
    onCreated?.(id, trimmed);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="lg">
      <DialogTitle>Add memory</DialogTitle>
      <DialogDescription>
        {agentId
          ? `A durable note ${agentName ?? "this teammate"} keeps between conversations.`
          : "A durable note one of your teammates keeps between conversations."}
      </DialogDescription>

      <DialogBody>
        <ScrollArea className="max-h-[min(28rem,calc(100dvh-16rem))] -mx-1 px-1">
          <div className="flex flex-col gap-4 pb-1">
            <div className="grid gap-3 sm:grid-cols-[10rem_1fr]">
              <Field label="Kind">
                <Select
                  size="xs"
                  ariaLabel="Memory kind"
                  value={kind}
                  options={KIND_OPTIONS}
                  onChange={setKind}
                />
              </Field>
              {agentId ? (
                <Field label="Title">
                  <Input
                    size="xs"
                    autoFocus
                    value={title}
                    onChange={(e) => setTitle(e.target.value)}
                    placeholder="Briefs are 400 words, hard cap"
                    aria-label="Memory title"
                  />
                </Field>
              ) : (
                <Field label="Agent">
                  <Select
                    size="xs"
                    ariaLabel="Agent"
                    value={agent}
                    options={agents.map((b) => ({ value: b.id, label: b.name, description: b.role }))}
                    onChange={setAgent}
                  />
                </Field>
              )}
            </div>

            {agentId ? null : (
              <Field label="Title">
                <Input
                  size="xs"
                  autoFocus
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="Briefs are 400 words, hard cap"
                  aria-label="Memory title"
                />
              </Field>
            )}

            <Field label="Body">
              <Textarea
                autoResize
                maxRows={16}
                rows={6}
                className="text-[13px] leading-relaxed"
                value={body}
                onChange={(e) => setBody(e.target.value)}
                placeholder="What happened, what the rule is, and why it matters."
                aria-label="Memory body"
              />
            </Field>

            <Field label="Tags">
              <div className="flex flex-wrap items-center gap-1.5">
                {tags.map((tag) => (
                  <span
                    key={tag}
                    className="inline-flex h-6 items-center gap-1 rounded-full fill-secondary pr-1 pl-2.5 text-[11px] text-muted-foreground"
                  >
                    {tag}
                    <button
                      type="button"
                      aria-label={`Remove ${tag}`}
                      onClick={() => removeTag(tag)}
                      className={cn(
                        "grid size-4 place-items-center rounded-full outline-none",
                        "transition-colors duration-150 ease-out hover:fill-close hover:text-foreground",
                        "focus-visible:fill-close"
                      )}
                    >
                      <X className="size-3" aria-hidden="true" />
                    </button>
                  </span>
                ))}
                <Input
                  size="xs"
                  className="w-40"
                  placeholder="Add a tag"
                  aria-label="Add a tag"
                  value={tagDraft}
                  onChange={(e) => setTagDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === ",") {
                      e.preventDefault();
                      addTag(tagDraft);
                    } else if (e.key === "Backspace" && !tagDraft && tags.length) {
                      e.preventDefault();
                      removeTag(tags[tags.length - 1]);
                    }
                  }}
                  onBlur={() => addTag(tagDraft)}
                />
              </div>
            </Field>

            <div className="flex h-7 items-center gap-2">
              <span className="text-xs text-foreground/90">Pinned</span>
              <p className="min-w-0 flex-1 text-[11px] text-muted-foreground">
                Pinned memories are always loaded into context.
              </p>
              <Switch checked={pinned} onCheckedChange={setPinned} aria-label="Pinned" />
            </div>
          </div>
        </ScrollArea>
      </DialogBody>

      <DialogFooter>
        <Button variant="secondary" size="pill" className="w-full sm:w-auto" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button
          variant="primary"
          size="pill"
          className="w-full sm:w-auto"
          disabled={!trimmed || !agent}
          onClick={submit}
        >
          Add memory
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

function Field({ label, children }) {
  // a <label> would forward its click into the Select's trigger and reopen it
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-xs text-foreground/90">{label}</span>
      {children}
    </div>
  );
}
