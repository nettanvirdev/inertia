import * as React from "react";
import { FolderOpen, ShieldQuestion, TriangleAlert } from "@/components/icons";
import { cn } from "@/lib/utils";
import { toolByKey } from "@shared/tools";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Textarea } from "@/components/ui/textarea";
import { ToolDiff } from "@/features/chat/ToolDiff";

/**
 * The permission ask.
 *
 * It is a card IN the transcript, not a dialog over it. A modal would be the
 * easy build and the wrong one: the whole basis for the decision is what the
 * agent said and did in the last few turns, and a modal is precisely the thing
 * that stops you scrolling back to read it. So this sits in the flow, the
 * transcript stays live behind and above it, and the reader can go and look
 * before answering.
 *
 * The second rule this card is built around: never ask someone to agree to a
 * scope they cannot see. "Always allow" carries the pattern it will write into
 * the ruleset, in the button, in monospace. A person who clicks it has read
 * `git push *` and meant it.
 */

const DANGER_INK = {
  high: "text-destructive-ink",
  medium: "text-warning-ink",
  low: "text-muted-foreground",
};

const DANGER_WORD = {
  high: "High risk",
  medium: "Medium risk",
  low: "Low risk",
};

function Dangers({ dangers }) {
  return (
    <ul className="flex flex-col gap-1">
      {dangers.map((danger, i) => (
        <li
          key={i}
          className="flex items-start gap-2 rounded-xl bg-warning-wash px-3 py-2 text-[12px] leading-relaxed text-warning-ink"
        >
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1">{danger.why ?? String(danger)}</span>
        </li>
      ))}
    </ul>
  );
}

function Directories({ directories }) {
  return (
    <div className="flex flex-col gap-1">
      <p className="text-[11px] text-muted-foreground">Outside the working folder</p>
      {directories.map((dir) => (
        <p
          key={dir}
          className="flex items-start gap-2 font-mono text-[12px] break-all text-foreground"
        >
          <FolderOpen
            className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
            aria-hidden="true"
          />
          <span className="min-w-0 flex-1">{dir}</span>
        </p>
      ))}
    </div>
  );
}

export function PermissionPrompt({ question, onReply, className }) {
  const [refusing, setRefusing] = React.useState(false);
  const [note, setNote] = React.useState("");
  const allowRef = React.useRef(null);
  const noteRef = React.useRef(null);

  const meta = toolByKey(question?.key);
  const danger = meta?.danger ?? "medium";
  const high = danger === "high";
  const metadata = question?.metadata ?? {};
  const dangers = Array.isArray(metadata.dangers) ? metadata.dangers : [];
  const directories = Array.isArray(metadata.directories) ? metadata.directories : [];
  const pattern = question?.always;

  // A composite session id means a subagent is asking. The user delegated the
  // work but did not choose this call, so the card has to say who wants it -
  // otherwise the request looks like it came from the agent they are talking to.
  const delegate = String(question?.sessionId ?? "").includes("/") ? question?.agent : null;

  // The safest answer holds the focus. "Always" is one keystroke further away
  // on purpose - a reflex Enter should grant the smallest thing that unblocks
  // the turn, never a standing rule.
  React.useEffect(() => {
    allowRef.current?.focus();
  }, []);

  const reply = React.useCallback(
    (answer, message = "") => onReply?.(question.id, answer, message),
    [onReply, question?.id]
  );

  function handleKeyDown(event) {
    if (event.key === "Escape") {
      event.preventDefault();
      reply("reject", note.trim());
      return;
    }
    if (event.key !== "Enter" || event.shiftKey) return;
    // Enter inside the feedback box sends the refusal it belongs to, not an
    // allow - the field only exists because the user already chose to refuse.
    if (event.target === noteRef.current) {
      event.preventDefault();
      reply("reject", note.trim());
      return;
    }
    if (event.target instanceof HTMLButtonElement) return;
    event.preventDefault();
    reply("once");
  }

  if (!question) return null;

  return (
    <div
      data-permission-prompt={question.key}
      role="group"
      aria-label="Permission request"
      onKeyDown={handleKeyDown}
      /* No outline. This card used to draw a hairline around itself and a
         second, redder one when the risk was high - and it sits inside a
         transcript whose messages, tool cards and diffs are all separated by a
         step of colour and nothing else, so the one thing on the page with a
         box drawn around it read as a dialog that had failed to open. The
         surface does the separating, which is what `card-surface-subtle` is
         for; a high-risk ask is coloured rather than fenced, and the icon, the
         wash and the words "High risk" say it three times over. */
      className={cn(
        "my-3 flex flex-col gap-3 rounded-2xl p-4",
        high ? "bg-destructive-wash" : "card-surface-subtle",
        className
      )}
    >
      <div className="flex items-start gap-2.5">
        <ShieldQuestion
          aria-hidden="true"
          className={cn("mt-0.5 size-4 shrink-0", DANGER_INK[danger] ?? DANGER_INK.medium)}
        />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex flex-wrap items-baseline gap-x-2">
            <p className="text-[13px] font-medium text-foreground">
              {question.title ?? meta?.label ?? "This agent wants permission"}
            </p>
            <span className={cn("text-[11px]", DANGER_INK[danger] ?? DANGER_INK.medium)}>
              {DANGER_WORD[danger] ?? DANGER_WORD.medium}
            </span>
            {delegate ? (
              <span className="text-[11px] text-muted-foreground">asked by {delegate}</span>
            ) : null}
          </div>
          {meta?.description ? (
            <p className="text-[12px] leading-relaxed text-muted-foreground">{meta.description}</p>
          ) : null}
        </div>
      </div>

      {/* The target is the decision. It gets its own line, at full weight, and
          it wraps rather than truncates - a half-shown path is a half-made
          decision. */}
      {question.target ? (
        <p className="rounded-xl fill-whisper px-3 py-2 font-mono text-[12px] leading-relaxed break-all text-foreground">
          {question.target}
        </p>
      ) : null}

      {directories.length ? <Directories directories={directories} /> : null}
      {dangers.length ? <Dangers dangers={dangers} /> : null}
      {metadata.diff ? <ToolDiff diff={metadata.diff} /> : null}

      <div className="flex flex-wrap items-center gap-2">
        <Button ref={allowRef} variant="primary" size="sm" onClick={() => reply("once")}>
          Allow once
          <Kbd className="bg-background/20 text-background/80">↵</Kbd>
        </Button>
        {/* The pattern is the point of this button - it is what "always"
            will remember - and a shell command is as long as it is. A button
            one line tall with the pattern nailed to that line ran straight
            out of the card. This one may be as tall as its pattern needs:
            the text wraps at any character, since a command has no natural
            break points, and the button takes the width of the row rather
            than pushing past it.

            No pattern, no button. Some calls are asked about every time on
            purpose - starting a program an agent configured, touching the
            secrets - and a button there would promise a rule that is never
            written. */}
        {pattern ? (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => reply("always")}
            className="h-auto min-h-[1.875rem] min-w-0 max-w-full shrink items-start whitespace-normal py-1 text-left"
          >
            <span className="shrink-0 py-0.5">Always allow</span>
            <code className="min-w-0 max-w-full rounded-[6px] fill-secondary px-1.5 py-0.5 font-mono text-[11px] break-all text-foreground">
              {pattern}
            </code>
          </Button>
        ) : null}
        <Button
          variant="danger"
          size="sm"
          onClick={() => {
            if (refusing) reply("reject", note.trim());
            else setRefusing(true);
          }}
        >
          Refuse
          <Kbd className="bg-transparent opacity-70">Esc</Kbd>
        </Button>
      </div>

      {/* Refusing with a reason redirects the turn instead of ending it: "no,
          use the staging bucket" is worth far more to the model than a bare
          denial, and it costs one line to offer. */}
      {refusing ? (
        <div className="flex flex-col gap-2">
          <label htmlFor={`${question.id}-note`} className="text-[11px] text-muted-foreground">
            Tell the agent what to do instead (optional)
          </label>
          <Textarea
            id={`${question.id}-note`}
            ref={noteRef}
            autoFocus
            rows={2}
            autoResize
            maxRows={6}
            value={note}
            onChange={(event) => setNote(event.target.value)}
            placeholder="No - use the staging bucket instead."
          />
          <div className="flex items-center gap-2">
            <Button variant="danger" size="sm" onClick={() => reply("reject", note.trim())}>
              Send refusal
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setRefusing(false)}>
              Cancel
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
