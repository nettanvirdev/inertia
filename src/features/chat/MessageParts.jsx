import * as React from "react";
import { ChevronRight, Info } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Collapse } from "@/components/ui/collapse";
import { Markdown } from "@/features/chat/Markdown";
import { ToolCallCard, ToolRun } from "@/features/chat/ToolCallCard";

/**
 * The body of a reply: prose, thinking, tool cards, in the order they happened.
 *
 * Its own file because there are two places a reply is read. The transcript is
 * the obvious one. The other is a helper's run in the Agents column, and the
 * whole point of that column is that watching a helper work should look like
 * watching the agent work - the same markdown, the same tool cards, the same
 * folded runs of reads. Two renderers would have meant two, drifting apart one
 * fix at a time.
 *
 * It takes items rather than parts, because grouping consecutive reads is a
 * decision about the whole list (see `tool-groups.js`) and the caller already
 * has to make it to know what the last item is.
 */

/**
 * @param {boolean} live  whether the model is thinking right now, as opposed to
 *   having thought earlier in a reply that has since moved on.
 */
function Reasoning({ text, live = false, startedAt, endedAt }) {
  const [open, setOpen] = React.useState(false);
  if (!text?.trim()) return null;

  // Only for a thought that has finished and was long enough to have been
  // waited through. Under a second is not a wait, and "Thought for 0s" is a
  // number pretending to be information.
  const seconds =
    !live && startedAt && endedAt ? Math.round((endedAt - startedAt) / 1000) : 0;
  const label = live
    ? "Thinking"
    : seconds >= 1
      ? `Thought for ${seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`}`
      : "Thought";

  return (
    <div className="my-1.5">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={cn(
          "flex items-center gap-1.5 rounded-md px-1.5 py-0.5 -mx-1.5 text-[12px] outline-none",
          "text-muted-foreground transition-colors duration-150 hover:text-foreground",
          "focus-visible:fill-control-hover"
        )}
      >
        <ChevronRight
          className={cn(
            "size-3 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
            open && "rotate-90"
          )}
        />
        {/* A light travelling across the word while the thought is still
            arriving, and plain text once it has stopped. Thinking is the one
            part of a reply with nothing to show for itself - no tokens, no
            card, no progress of any kind - so the label is the only place the
            difference between working and finished can be drawn at all. It
            goes on the label rather than the row because the chevron beside it
            is a control, and a control that shimmers reads as broken. */}
        <span className={cn(live && !open && "text-shimmer")}>
          {open ? "Hide thinking" : label}
        </span>
      </button>
      <Collapse open={open}>
        {/* `wrap-anywhere`, not the usual `wrap-break-word`: thinking is full of
            things that are one word to a browser and half a kilobyte long - an
            OAuth callback URL, a base64 state parameter, a stack frame. Break-word
            leaves those on one line and they run out of the column, under the side
            panel and off the window, which is what this was doing. */}
        <p className="mt-1 whitespace-pre-wrap wrap-anywhere border-l border-border/60 pl-3 text-[12px] leading-relaxed text-muted-foreground">
          {text}
        </p>
      </Collapse>
    </div>
  );
}

export function MessageParts({ items, streaming = false, fallback }) {
  return (
    <>
  {items.length ? (
    items.map((item, index) =>
      item.kind === "run" ? (
        // Consecutive calls to one tool, folded into a lid. See
        // `tool-groups.js`; every member is still openable underneath.
        <ToolRun key={item.key} calls={item.calls} />
      ) : item.part.type === "tool" ? (
        <ToolCallCard key={item.key} part={item.part} />
      ) : item.part.type === "reasoning" ? (
        <Reasoning
          key={item.key}
          text={item.part.text}
          startedAt={item.part.startedAt}
          endedAt={item.part.endedAt}
          live={streaming && index === items.length - 1}
        />
      ) : item.part.type === "error" ? (
        <p key={item.key} className="my-2 text-[13px] text-destructive-ink">
          {item.part.text}
        </p>
      ) : item.part.type === "steer" ? (
        // The person, in the middle of the reply. Drawn as their own
        // bubble and pushed to their side of the conversation, because
        // that is what it is - and because a correction rendered as
        // another line of the agent's prose reads as the agent agreeing
        // with itself. It sits between the card they were watching and
        // whatever the agent did about it, which is where they said it.
        <div key={item.key} className="my-2 flex justify-end">
          <div className="max-w-[80%] rounded-2xl rounded-br-md bg-user-message-background px-4 py-2.5 wrap-break-word">
            <Markdown>{item.part.text}</Markdown>
          </div>
        </div>
      ) : item.part.type === "notice" ? (
        // Not an error - the turn is still going. A rate limit being
        // waited out, or a long thread being compacted: both take real
        // seconds, and a turn that pauses with nothing on screen is
        // indistinguishable from one that has hung.
        <p
          key={item.key}
          className="my-2 flex items-start gap-1.5 text-[12px] text-muted-foreground"
        >
          <Info className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          <span>{item.part.text}</span>
        </p>
      ) : (
        <Markdown key={item.key} streaming={streaming && index === items.length - 1}>
          {item.part.text}
        </Markdown>
      )
    )
  ) : (
    /* the caret belongs inside the renderer: appended out here it lands
       below the prose whenever the reply ends in a block element */
    <Markdown streaming={streaming}>{fallback ?? ""}</Markdown>
  )}
    </>
  );
}
