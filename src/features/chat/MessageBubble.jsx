import * as React from "react";
import {
  Copy,
  RotateCcw,
  Square,
  AudioLines,
  ThumbsDown,
  ThumbsUp,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { relativeTime } from "@/data";
import { appearanceOf } from "@/lib/appearance";
import { copyText } from "@/lib/clipboard";
import { textOf } from "@/lib/agent";
import { IconButton } from "@/components/ui/icon-button";
import { Button } from "@/components/ui/button";
import { useToast } from "@/components/ui/toast";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { Markdown } from "@/features/chat/Markdown";
import { MessageParts } from "@/features/chat/MessageParts";
import { stripSelfLabel, stripSelfLabelParts } from "@/features/chat/self-label";
import { groupParts } from "@/features/chat/tool-groups";
import { ChangesStrip } from "@/features/chat/ChangesStrip";
import { FileChip, FilePreview } from "@/features/chat/FilePreview";
import { useArrival } from "@/features/chat/arrival";
import { useSpeakMarkdown, useSpeechState } from "@/hooks/use-speech";

/**
 * One message in the transcript.
 *
 * The user gets a bubble; the agent does not. That asymmetry is deliberate - an
 * answer is content sitting on the page, not a chat message in a container.
 * The only "this is mine" signal on the user side is the squared bottom-right
 * corner, in place of a colour.
 */

/**
 * What was attached, shown rather than named.
 *
 * An attached image is a thumbnail, and everything is a button: clicking any
 * of them opens the file at a size worth looking at. This used to be a row of
 * pills reading "screenshot.png  84 KB", which told the reader the one thing
 * about a screenshot that does not matter.
 */
function Attachments({ items }) {
  const [open, setOpen] = React.useState(null);
  if (!items?.length) return null;
  return (
    <>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {items.map((file) => (
          <FileChip key={file.id} file={file} onOpen={setOpen} />
        ))}
      </div>
      <FilePreview file={open} onClose={() => setOpen(null)} />
    </>
  );
}

/** Reserved-height row so revealing it on hover never moves the transcript. */
function MessageActions({ onCopy, onRetry, align = "start", speak = null }) {
  const { toast } = useToast();
  return (
    <div
      className={cn(
        "mt-0.5 flex h-7 items-center gap-0.5 transition-opacity duration-150 ease-out",
        // The row hides until hover, except while this reply is the one being
        // read out: a Stop button you have to find the right message and hover
        // to reach is not a stop button.
        speak?.active ? "opacity-100" : "opacity-0",
        "group-hover:opacity-100 group-focus-within:opacity-100",
        align === "end" && "justify-end"
      )}
    >
      <IconButton size="sm" label="Copy message" onClick={onCopy}>
        <Copy />
      </IconButton>
      {speak ? (
        <IconButton
          size="sm"
          label={speak.active ? "Stop reading this reply" : "Read this reply aloud"}
          className={cn(speak.active && "text-foreground")}
          onClick={speak.onToggle}
        >
          {speak.active ? <Square /> : <AudioLines />}
        </IconButton>
      ) : null}
      <IconButton size="sm" label="Retry" onClick={onRetry}>
        <RotateCcw />
      </IconButton>
      <IconButton
        size="sm"
        label="Good response"
        onClick={() => toast({ title: "Thanks - noted as a good response." })}
      >
        <ThumbsUp />
      </IconButton>
      <IconButton
        size="sm"
        label="Bad response"
        onClick={() => toast({ title: "Thanks - noted as a bad response." })}
      >
        <ThumbsDown />
      </IconButton>
    </div>
  );
}

/**
 * What the model thought before it answered.
 *
 * Collapsed by default and quiet, because it is not the answer. Worth showing
 * at all because a reasoning model that reached a strange conclusion usually
 * shows you where it went wrong, and because a long silent pause before the
 * first token reads as the app being broken.
 */
/** The time under a message, or nothing if the reader turned times off. */
function Timestamp({ at, show, align = "start" }) {
  if (!show || !at) return null;
  return (
    <p
      className={cn(
        "mt-1 text-[11px] text-muted-foreground",
        align === "end" && "text-right"
      )}
    >
      {relativeTime(at)}
    </p>
  );
}

/**
 * One message.
 *
 * Memoised, and the three props that look like they could have been read from
 * the store are the reason. This component used to call `useApp()`, which
 * subscribes it to the whole application state - including `messages`, which
 * changes identity on every token of every streaming reply. So every message
 * on screen re-rendered several times a second while an agent typed, each one
 * re-running its markdown parse, its syntax highlighting and its diff. Thirty
 * of those is what a long conversation felt like.
 *
 * Passed in, the props are stable, the memo holds, and a streaming reply
 * re-renders exactly one bubble: its own.
 */
function MessageBubbleInner({
  message,
  agent,
  isLast,
  threadId,
  retryMessage,
  preferences,
  className,
}) {
  const { toast } = useToast();

  // Which reply the shared speaker is on, so exactly one message shows Stop.
  //
  // Called up here rather than beside the reply that uses them, because a
  // system or user message returns before that point and a hook after an early
  // return is called on some renders and not others. It survived only because
  // a message never changes role once it exists, which is a property of the
  // data rather than a guarantee this component was making.
  const speech = useSpeakMarkdown();
  const speakerState = useSpeechState();

  // Read through `appearanceOf` rather than off the bag directly, so a profile
  // written before these preferences existed still gets the shipped answer
  // instead of `undefined` quietly reading as "off".
  const { showAvatars, showTimestamps } = appearanceOf(preferences);

  // A message sent or begun while the person is watching rises into place.
  // One restored with the conversation, revealed by "older messages", or
  // mounted again after a thread switch is already there and holds still -
  // see arrival.js. Decided once at mount, so a streaming reply's tokens do
  // not restart it, and the class is dropped once the motion is over.
  const arrival = useArrival(message.id, { at: message.createdAt });
  const entrance = arrival.arriving && "animate-slide-up";
  // A failure that lands on a reply already on screen fades in under it; a
  // message restored already failed mounts with its failure and holds still.
  // Its own decision rather than the message's, because the reply's entrance
  // is long over by the time a turn fails. Up here with the other hooks for
  // the same reason `speech` is: the early returns below come first for a
  // user message, and a hook after them is called on some renders and not
  // others.
  const failure = useArrival(null, { live: message.status !== "error" });

  // A streaming reply keeps its text in `parts` and only gains `content` when
  // the turn ends, so copying the bag's `content` alone copies nothing from a
  // message that is still arriving - and the toast used to say otherwise.
  const copy = async () => {
    const text = message.content || textOf(message);
    if (await copyText(text)) {
      toast({ variant: "success", title: "Copied to clipboard" });
      return;
    }
    toast({
      variant: "warning",
      title: text ? "Could not copy" : "Nothing to copy",
      description: text ? "This window has no access to the clipboard." : undefined,
    });
  };

  /**
   * The reply's parts, with runs of reads folded into one row each.
   *
   * Memoised because it runs on every token of a streaming reply, and because
   * the identity of the items is what keeps React from remounting the cards
   * above the one being appended to.
   */
  // A reply that opens with its own name - "Folder Organizer: Hi" - loses the
  // name. It is on the bubble, beside the face; see self-label.js.
  const items = React.useMemo(
    () => groupParts(stripSelfLabelParts(message.parts, agent?.name)),
    [message.parts, agent?.name]
  );

  // Retry asks the same agent again from where it started, dropping the answer
  // that failed. It does not re-send what the person typed: that would leave a
  // second copy of their own message in the transcript, and in a group it would
  // hand the floor back to the whole room and replay the entire conversation.
  const retry = () => {
    if (!retryMessage(threadId, message.id)) {
      toast({ variant: "warning", title: "Nothing to retry" });
    }
  };

  // A turn that had nothing to add. One line, so the reader can see why the
  // conversation moved on to somebody else, and no bubble, because there is
  // nothing in it. See the quiet-turn note in the store.
  if (message.quiet) {
    return (
      <div
        onAnimationEnd={arrival.onAnimationEnd}
        className={cn("py-1 pl-6", arrival.arriving && "animate-fade-in", className)}
      >
        <p className="text-[11px] text-muted-foreground">
          {agent?.name ? `${agent.name} passed` : "Passed"}
        </p>
      </div>
    );
  }

  if (message.role === "system") {
    return (
      <div
        onAnimationEnd={arrival.onAnimationEnd}
        className={cn("my-4 px-6 text-center", entrance, className)}
      >
        <p className="text-[11px] leading-relaxed text-muted-foreground">{message.content}</p>
      </div>
    );
  }

  if (message.role === "user") {
    return (
      <div
        onAnimationEnd={arrival.onAnimationEnd}
        className={cn("group flex flex-col items-end py-2", entrance, className)}
      >
        {/* Two nested caps rather than one: 80% on the wrapper keeps the
            asymmetry that says which side of the conversation this is even at
            "Full width", and chat-measure on the bubble itself is the reader's
            chosen line length. Both on one element would collide. */}
        <div className="flex max-w-[80%] flex-col items-end">
          {/* Rendered, not printed. People paste briefs, checklists and code
              into this box, and a plain text node collapsed every newline: a
              structured request came back as one grey slab with `##` and `*`
              still in it, unreadable next to the answer it produced. The same
              renderer as the reply, so both halves of the conversation are
              read the same way. */}
          <div className="chat-measure rounded-2xl rounded-br-md bg-user-message-background px-4 py-2.5 wrap-break-word">
            <Markdown>{message.content}</Markdown>
          </div>
          <Timestamp at={message.createdAt} show={showTimestamps} align="end" />
        </div>
        <Attachments items={message.attachments} />
        <MessageActions onCopy={copy} onRetry={retry} align="end" />
      </div>
    );
  }

  const streaming = message.status === "streaming";
  const speaking = speakerState.status !== "idle" && speakerState.id === message.id;
  const errored = message.status === "error";

  return (
    <div
      onAnimationEnd={arrival.onAnimationEnd}
      className={cn("group flex flex-col py-2", entrance, className)}
    >
      <div className="mb-1.5 flex items-center gap-2">
        {agent && showAvatars ? <AgentAvatar agent={agent} size="xs" /> : null}
        <span className="text-[13px] font-medium text-foreground">{agent?.name ?? "Assistant"}</span>
        {isLast && streaming ? (
          <span className="text-[11px] text-muted-foreground">typing…</span>
        ) : null}
      </div>

      <div className="chat-measure min-w-0">
        {/* A reply that used tools is a sequence rather than a block of prose:
            what it said, what it did, what it said next. Rendering `parts` in
            order is what keeps the tool cards where they actually happened
            instead of collecting them all at the top or the bottom.

            A message with no parts is one from before tools existed, or one
            from the browser preview where there are none. It still renders. */}
        <MessageParts
          items={items}
          streaming={streaming}
          fallback={stripSelfLabel(message.content, agent?.name)}
        />
        {/* What the folder looks like after all of the above. Once, at the
            end, with the way back: the cards say what was asked, this says
            what happened. */}
        {message.changes?.files?.length ? <ChangesStrip changes={message.changes} /> : null}
      </div>

      <Attachments items={message.attachments} />
      <Timestamp at={message.createdAt} show={showTimestamps && !streaming} />

      {errored ? (
        <div
          onAnimationEnd={failure.onAnimationEnd}
          className={cn("mt-2 flex items-center gap-2", failure.arriving && "animate-fade-in")}
        >
          <p className="text-[13px] text-destructive-ink">
            {message.error ?? "The response could not be completed."}
          </p>
          <Button variant="ghost" size="xs" onClick={retry}>
            <RotateCcw />
            Retry
          </Button>
        </div>
      ) : null}

      {streaming ? (
        <div className="h-7" />
      ) : (
        <MessageActions
          onCopy={copy}
          onRetry={retry}
          speak={{
            active: speaking,
            onToggle: () =>
              speech.toggle(message.id, message.content).catch((failure) =>
                toast({
                  variant: "danger",
                  title: "Could not read that aloud",
                  description: failure.message,
                })
              ),
          }}
        />
      )}
    </div>
  );
}

export const MessageBubble = React.memo(MessageBubbleInner);
